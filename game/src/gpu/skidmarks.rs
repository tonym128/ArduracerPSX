//! Persistent skidmark buffer for tire drift marks.
//!
//! Stores skidmark segments laid down by tires during drift in a circular
//! buffer and renders them into the frame buffer prior to vehicles.

use crate::gpu::camera::Camera;
use arduracer_core::{math, Fixed, Vec2};
use psx_gpu as gpu;

pub const MAX_SKIDMARKS: usize = 96;

#[derive(Copy, Clone, Debug, Default)]
pub struct Skidmark {
    pub active: bool,
    pub left_pos: Vec2,
    pub right_pos: Vec2,
    pub life: u8,
}

pub struct SkidmarkBuffer {
    pub marks: [Skidmark; MAX_SKIDMARKS],
    pub head: usize,
}

impl SkidmarkBuffer {
    pub const fn new() -> Self {
        SkidmarkBuffer {
            marks: [Skidmark {
                active: false,
                left_pos: Vec2::ZERO,
                right_pos: Vec2::ZERO,
                life: 0,
            }; MAX_SKIDMARKS],
            head: 0,
        }
    }

    /// Emits a pair of tire marks behind the rear wheels based on vehicle position and heading.
    pub fn emit(&mut self, pos: Vec2, angle: u16) {
        let cos_a = math::cos(angle);
        let sin_a = math::sin(angle);

        // Vector perpendicular to heading for wheel track width (8 pixels = 8 * 4096)
        let lateral_x = (cos_a * Fixed::from_int(7)).raw();
        let lateral_y = (sin_a * Fixed::from_int(7)).raw();

        // Vector rearward from center (-10 pixels = -10 * 4096)
        let rear_x = (-sin_a * Fixed::from_int(10)).raw();
        let rear_y = (cos_a * Fixed::from_int(10)).raw();

        let base_rear = Vec2::new(
            pos.x + Fixed::from_raw(rear_x),
            pos.y + Fixed::from_raw(rear_y),
        );

        let left = Vec2::new(
            base_rear.x - Fixed::from_raw(lateral_x),
            base_rear.y - Fixed::from_raw(lateral_y),
        );
        let right = Vec2::new(
            base_rear.x + Fixed::from_raw(lateral_x),
            base_rear.y + Fixed::from_raw(lateral_y),
        );

        self.marks[self.head] = Skidmark {
            active: true,
            left_pos: left,
            right_pos: right,
            life: 180, // Persist for ~3 seconds at 60 FPS
        };
        self.head = (self.head + 1) % MAX_SKIDMARKS;
    }

    /// Decrements lifetime of skidmarks.
    pub fn tick(&mut self) {
        for m in self.marks.iter_mut() {
            if m.active {
                if m.life > 0 {
                    m.life -= 1;
                }
                if m.life == 0 {
                    m.active = false;
                }
            }
        }
    }

    /// Renders skidmark stamps to screen relative to camera.
    pub fn render(&self, camera: &Camera, draw_y: i16) {
        for m in self.marks.iter() {
            if !m.active {
                continue;
            }

            let (lx, ly) = camera.world_to_screen(m.left_pos, draw_y);
            if lx >= 0 && lx < 320 && ly >= 0 && ly < 240 + draw_y {
                gpu::fill_rect(lx as u16, ly as u16, 2, 2, 22, 22, 26);
            }

            let (rx, ry) = camera.world_to_screen(m.right_pos, draw_y);
            if rx >= 0 && rx < 320 && ry >= 0 && ry < 240 + draw_y {
                gpu::fill_rect(rx as u16, ry as u16, 2, 2, 22, 22, 26);
            }
        }
    }
}
