//! Pure parts of the native Windows multiplexer (feature 34): no I/O, no OS calls, so they are
//! built and tested on every platform. The Windows glue (ConPTY, named pipe) lives behind
//! `cfg(windows)` elsewhere and only moves bytes between these pieces and the OS.

pub mod frame;
pub mod screen;
pub mod server;
