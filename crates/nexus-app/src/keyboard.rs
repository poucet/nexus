//! An on-screen / computer-keyboard MIDI keyboard, so you can play a hosted
//! plugin without any MIDI hardware.
//!
//! Computer keys map a piano octave (A W S E D F T G Y H U J K = C…C); on-screen
//! keys are clickable. The widget tracks which notes are held and emits note
//! on/off transitions for the app to forward to the engine.

use std::collections::HashSet;

use eframe::egui::{self, Key};

/// Computer key → semitone offset within the current octave.
const KEY_MAP: &[(Key, i32)] = &[
    (Key::A, 0),
    (Key::W, 1),
    (Key::S, 2),
    (Key::E, 3),
    (Key::D, 4),
    (Key::F, 5),
    (Key::T, 6),
    (Key::G, 7),
    (Key::Y, 8),
    (Key::H, 9),
    (Key::U, 10),
    (Key::J, 11),
    (Key::K, 12),
];

const NOTE_NAMES: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", "C",
];

/// A note-on or note-off the app should forward to the engine.
pub enum KeyEvent {
    On(u8),
    Off(u8),
}

pub struct VirtualKeyboard {
    octave: i32,
    held: HashSet<u8>,
}

impl VirtualKeyboard {
    pub fn new() -> Self {
        Self { octave: 4, held: HashSet::new() }
    }

    /// Draw the keyboard and return note on/off transitions since last frame.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Vec<KeyEvent> {
        ui.horizontal(|ui| {
            ui.label("Keyboard");
            if ui.button("octave −").clicked() {
                self.octave = (self.octave - 1).clamp(0, 8);
            }
            ui.label(format!("C{}", self.octave));
            if ui.button("octave +").clicked() {
                self.octave = (self.octave + 1).clamp(0, 8);
            }
            ui.separator();
            ui.weak("play: A W S E D F T G Y H U J K");
        });

        let base = 12 * (self.octave + 1); // C4 == MIDI 60
        let mut pressed: HashSet<u8> = HashSet::new();

        // Computer keyboard (held keys).
        ui.input(|input| {
            for (key, offset) in KEY_MAP {
                if input.keys_down.contains(key) {
                    pressed.insert((base + offset).clamp(0, 127) as u8);
                }
            }
        });

        // On-screen clickable keys (held while the pointer is down on them),
        // drawn like piano keys: light naturals, dark sharps.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (offset, name) in NOTE_NAMES.iter().enumerate() {
                let note = (base + offset as i32).clamp(0, 127) as u8;
                let is_sharp = matches!(offset % 12, 1 | 3 | 6 | 8 | 10);
                let (fill, text_color, width) = if is_sharp {
                    (egui::Color32::from_gray(35), egui::Color32::from_gray(220), 26.0)
                } else {
                    (egui::Color32::from_gray(235), egui::Color32::from_gray(20), 34.0)
                };
                let button = egui::Button::new(egui::RichText::new(*name).color(text_color))
                    .fill(fill)
                    .min_size(egui::vec2(width, 72.0));
                if ui.add(button).is_pointer_button_down_on() {
                    pressed.insert(note);
                }
            }
        });

        // Diff against the previously-held set to produce events.
        let mut events = Vec::new();
        for note in &pressed {
            if !self.held.contains(note) {
                events.push(KeyEvent::On(*note));
            }
        }
        for note in &self.held {
            if !pressed.contains(note) {
                events.push(KeyEvent::Off(*note));
            }
        }
        self.held = pressed;
        events
    }
}

impl Default for VirtualKeyboard {
    fn default() -> Self {
        Self::new()
    }
}
