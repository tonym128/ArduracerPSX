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

impl Default for SkidmarkBuffer {
    fn default() -> Self {
        Self::new()
    }
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
            let (rx, ry) = camera.world_to_screen(m.right_pos, draw_y);

            let (cr, cg, cb) = if m.life > 60 {
                (18, 18, 22) // Deep fresh black rubber
            } else {
                (28, 30, 36) // Faded rubber streak
            };

            if lx > -8 && lx < 328 && ly > -8 && ly < 248 {
                gpu::draw_rect_flat(lx - 1, ly - 1, 3, 3, cr, cg, cb);
            }
            if rx > -8 && rx < 328 && ry > -8 && ry < 248 {
                gpu::draw_rect_flat(rx - 1, ry - 1, 3, 3, cr, cg, cb);
            }
        }
    }
}
