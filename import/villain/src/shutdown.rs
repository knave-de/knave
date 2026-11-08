//! Graceful signal delivery for the compositor event loop.

use std::{
    fs::File,
    io::{self, Read},
    os::fd::FromRawFd,
    sync::atomic::{AtomicI32, Ordering},
};

use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};

use crate::state::Villain;

static SIGNAL_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

/// Wake calloop from SIGINT/SIGTERM without blocking those signals in clients.
pub fn install(
    event_loop: &mut EventLoop<'static, Villain>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fds = [-1; 2];
    let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }

    SIGNAL_WRITE_FD.store(fds[1], Ordering::SeqCst);
    install_handler(libc::SIGINT);
    install_handler(libc::SIGTERM);

    let read_end = unsafe { File::from_raw_fd(fds[0]) };
    let result = event_loop.handle().insert_source(
        Generic::new(read_end, Interest::READ, Mode::Level),
        |_, source, state| {
            let mut bytes = [0_u8; 16];
            let _ = unsafe { source.get_mut() }.read(&mut bytes);
            state.loop_signal.stop();
            Ok(PostAction::Remove)
        },
    );
    if let Err(error) = result {
        reset();
        return Err(error.into());
    }
    Ok(())
}

/// Restore defaults and close the signal-pipe writer after the event loop exits.
pub fn reset() {
    let fd = SIGNAL_WRITE_FD.swap(-1, Ordering::SeqCst);
    if fd >= 0 {
        unsafe {
            libc::close(fd);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
        }
    }
}

fn install_handler(signal: libc::c_int) {
    unsafe {
        libc::signal(signal, request_shutdown as *const () as libc::sighandler_t);
    }
}

extern "C" fn request_shutdown(signal: libc::c_int) {
    let fd = SIGNAL_WRITE_FD.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let byte = [signal as u8];
    unsafe {
        let _ = libc::write(fd, byte.as_ptr().cast(), byte.len());
    }
}
