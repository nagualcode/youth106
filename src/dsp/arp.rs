//! Juno-106 arpeggiator (Up / Up-Down / Down, 1-3 octaves).
//!
//! Ported from the Ultramaster KR-106 `KR106Arpeggiator.h`. Free-running
//! rate comes from the Schmitt-trigger relaxation oscillator model of the
//! ARPEGGIO RATE circuit; optional host sync locks steps to the beat grid.

/// Juno-106 keyboard top note (61 keys, C1-C6): arp notes transposed
/// above it fold down by octaves, like the hardware.
const MAX_ARP_NOTE: i32 = 96;

/// Beats per step for the sync divisions (1/1 .. 1/32).
pub const ARP_DIV_BEATS: [f64; 9] =
    [4.0, 2.0, 1.0, 2.0 / 3.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125];

/// ARP RATE slider -> steps per minute.
///
/// `f = 1 / (2 * (R21 + pos*VR1) * C4 * ln((1+a)/(1-a)))`, with
/// R21 = 33K, VR1 = 1M, C4 = 0.47uF, a = 47K/147K.
pub fn arp_rate_spm(t: f32) -> f32 {
    let pos = 1.0 - t;
    let hz = 1.0 / (2.0 * (33_000.0 + pos * 1_000_000.0) * 0.47e-6 * 0.6633);
    hz * 60.0
}

pub enum ArpEvent {
    On(u8),
    Off(u8),
}

#[derive(Clone)]
pub struct Arpeggiator {
    pub enabled: bool,
    /// 0 = Up, 1 = Up/Down, 2 = Down.
    pub mode: u32,
    /// 0 = 1 oct, 1 = 2 oct, 2 = 3 oct.
    pub range: u32,
    /// Steps per minute (free-running).
    pub rate: f32,
    pub sync: bool,
    pub division: usize,
    /// Sorted ascending; capacity reserved for all 128 notes up front so
    /// inserts never allocate on the audio thread.
    pub held: Vec<u8>,
    step: usize,
    direction: i32,
    phase: f32,
    pub last_note: Option<u8>,
    last_sync_step: i64,
}

impl Default for Arpeggiator {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: 0,
            range: 0,
            rate: 120.0,
            sync: false,
            division: 6,
            held: Vec::with_capacity(128),
            step: 0,
            direction: 1,
            phase: 0.0,
            last_note: None,
            last_sync_step: -1,
        }
    }
}

impl Arpeggiator {
    pub fn note_on(&mut self, note: u8) {
        if let Err(pos) = self.held.binary_search(&note) {
            self.held.insert(pos, note);
            if self.held.len() == 1 {
                // First note in: trigger on the next sample.
                self.phase = 1.0;
                self.step = 0;
                self.direction = 1;
                self.last_sync_step = -1;
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if let Ok(pos) = self.held.binary_search(&note) {
            self.held.remove(pos);
        }
        if self.held.is_empty() {
            self.step = 0;
            self.direction = 1;
            self.phase = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.held.clear();
        self.step = 0;
        self.direction = 1;
        self.phase = 0.0;
        self.last_note = None;
        self.last_sync_step = -1;
    }

    fn seq_len(&self) -> usize {
        self.held.len() * (self.range as usize + 1)
    }

    fn seq_note(&self, idx: usize) -> u8 {
        let n = self.held.len();
        let oct = idx / n;
        let mut note = i32::from(self.held[idx % n]) + 12 * oct as i32;
        while note > MAX_ARP_NOTE && note >= 12 {
            note -= 12;
        }
        note.clamp(0, 127) as u8
    }

    fn next_note(&mut self) -> Option<u8> {
        let len = self.seq_len();
        if len == 0 {
            return None;
        }
        if self.step >= len {
            self.step = 0;
        }
        let note = match self.mode {
            1 => {
                let n = self.seq_note(self.step);
                if len > 1 {
                    let next = self.step as i32 + self.direction;
                    if next >= len as i32 {
                        self.step = len - 2;
                        self.direction = -1;
                    } else if next < 0 {
                        self.step = 1;
                        self.direction = 1;
                    } else {
                        self.step = next as usize;
                    }
                }
                n
            }
            2 => {
                let n = self.seq_note(len - 1 - self.step);
                self.step = (self.step + 1) % len;
                n
            }
            _ => {
                let n = self.seq_note(self.step);
                self.step = (self.step + 1) % len;
                n
            }
        };
        Some(note)
    }

    fn step_event(&mut self, out: &mut [Option<ArpEvent>; 2]) {
        if let Some(n) = self.last_note.take() {
            out[0] = Some(ArpEvent::Off(n));
        }
        if let Some(n) = self.next_note() {
            out[1] = Some(ArpEvent::On(n));
            self.last_note = Some(n);
        }
    }

    /// Advance one sample. `host` = `(tempo, Some(beat_pos) if playing)`.
    /// Returns up to one note-off followed by one note-on.
    #[inline]
    pub fn process(
        &mut self,
        sample_rate: f32,
        host: (f64, Option<f64>),
    ) -> [Option<ArpEvent>; 2] {
        let mut out = [None, None];
        if !self.enabled || self.held.is_empty() {
            if let Some(n) = self.last_note.take() {
                out[0] = Some(ArpEvent::Off(n));
            }
            self.last_sync_step = -1;
            return out;
        }

        let inc = if self.sync {
            let div = ARP_DIV_BEATS[self.division.min(ARP_DIV_BEATS.len() - 1)];
            if let Some(beat) = host.1 {
                // Transport running: lock to the beat grid.
                let step_now = (beat / div).floor() as i64;
                if step_now != self.last_sync_step {
                    self.last_sync_step = step_now;
                    self.step_event(&mut out);
                }
                return out;
            }
            (host.0 / div) as f32 / (60.0 * sample_rate)
        } else {
            self.rate / (60.0 * sample_rate)
        };

        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.step_event(&mut out);
        }
        out
    }
}
