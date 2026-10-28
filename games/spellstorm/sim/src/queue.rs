//! The elements a wizard has conjured but not cast (spell_queue.gd). An
//! added element first melts with the first base element it fuses with,
//! otherwise it takes a new slot if there is room.

use crate::el::El;

pub const SIZE: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    Fuse,
    Add,
    Full,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Queue {
    pub slots: Vec<El>,
    pub size: usize,
}

impl Queue {
    pub fn new() -> Queue {
        Queue {
            slots: Vec::new(),
            size: SIZE,
        }
    }

    pub fn with(slots: &[El]) -> Queue {
        Queue {
            slots: slots.to_vec(),
            size: SIZE,
        }
    }

    pub fn add(&mut self, e: El) -> Added {
        for s in self.slots.iter_mut() {
            if !s.is_fused() {
                if let Some(f) = El::fuse(*s, e) {
                    *s = f;
                    return Added::Fuse;
                }
            }
        }
        if self.slots.len() >= self.size {
            return Added::Full;
        }
        self.slots.push(e);
        Added::Add
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
