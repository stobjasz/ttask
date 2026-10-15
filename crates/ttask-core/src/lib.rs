//! Domain types for ttask. This crate does no I/O.

mod clock;
mod task;

pub use clock::{Clock, FixedClock, SystemClock};
pub use task::{NewTask, Priority, Status, Task, TaskId, new_uid};
