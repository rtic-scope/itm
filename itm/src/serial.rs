//! Convenience module for serial device configuration.
//!
//! This module exposes a single function, [`configure`], used to
//! configure a serial device with a wanted baud rate so that the device
//! can be used with this crate. This functionality is used downstream
//! in `itm-decode` and `cargo-rtic-scope`.

use nix::{
    fcntl::{self, FcntlArg, OFlag},
    libc,
    sys::termios::{
        self, ArbitraryBaudRate, BaudRate, ControlFlags, InputFlags, LocalFlags, OutputFlags,
        SetArg, SpecialCharacterIndices as CC,
    },
};
use std::fs;
use std::os::unix::io::AsRawFd;
use thiserror::Error;

mod ioctl {
    use super::libc;
    use nix::{ioctl_none_bad, ioctl_read_bad, ioctl_write_int_bad, ioctl_write_ptr_bad};

    ioctl_none_bad!(tiocexcl, libc::TIOCEXCL);
    ioctl_read_bad!(tiocmget, libc::TIOCMGET, libc::c_int);
    ioctl_read_bad!(fionread, libc::FIONREAD, libc::c_int);
    ioctl_write_ptr_bad!(tiocmset, libc::TIOCMSET, libc::c_int);
    ioctl_write_int_bad!(tcflsh, libc::TCFLSH);
}

/// Possible errors on [`configure`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SerialError {
    #[error("Error configuring serial device: {0}")]
    General(String),
}

/// Opens and configures the given `device`.
///
/// Effectively mirrors the behavior of
/// ```shell,ignore
/// $ screen <device> <baud rate>
/// ```
///
/// TODO ensure POSIX compliance, see termios(3)
/// TODO We are currently using line disciple 0. Is that correct?
pub fn configure(device: &fs::File, baud_rate: u32) -> Result<(), SerialError> {
    use SerialError as Error;

    // ensure a valid baud rate was requested
    let baud_rate: BaudRate = ArbitraryBaudRate(baud_rate)
        .try_into()
        .map_err(|_| Error::General(format!("{} is not a valid baud rate", baud_rate)))?;
    if baud_rate == BaudRate::B0 {
        return Err(Error::General("baud rate cannot be 0".to_string()));
    }

    unsafe {
        let fd = device.as_raw_fd();

        // Enable exclusive mode. Any further open(2) will fail with EBUSY.
        ioctl::tiocexcl(fd).map_err(|e| {
            Error::General(format!(
                "Failed to put device into exclusive mode: tiocexcl = {}",
                e
            ))
        })?;

        let mut settings = termios::tcgetattr(fd).map_err(|e| {
            Error::General(format!(
                "Failed to read terminal settings of device: tcgetattr = {}",
                e
            ))
        })?;

        settings.input_flags |= InputFlags::BRKINT | InputFlags::IGNPAR | InputFlags::IXON;
        settings.input_flags &= !(InputFlags::ICRNL
            | InputFlags::IGNBRK
            | InputFlags::PARMRK
            | InputFlags::INPCK
            | InputFlags::ISTRIP
            | InputFlags::INLCR
            | InputFlags::IGNCR
            | InputFlags::ICRNL
            | InputFlags::IXOFF
            | InputFlags::IXANY
            | InputFlags::IMAXBEL
            | InputFlags::IUTF8);

        settings.output_flags |= OutputFlags::NL0
            | OutputFlags::CR0
            | OutputFlags::TAB0
            | OutputFlags::BS0
            | OutputFlags::VT0
            | OutputFlags::FF0;
        settings.output_flags &= !(OutputFlags::OPOST
            | OutputFlags::ONLCR
            | OutputFlags::OLCUC
            | OutputFlags::OCRNL
            | OutputFlags::ONOCR
            | OutputFlags::ONLRET
            | OutputFlags::OFILL
            | OutputFlags::OFDEL
            | OutputFlags::NL1
            | OutputFlags::CR1
            | OutputFlags::CR2
            | OutputFlags::CR3
            | OutputFlags::TAB1
            | OutputFlags::TAB2
            | OutputFlags::TAB3
            | OutputFlags::XTABS
            | OutputFlags::BS1
            | OutputFlags::VT1
            | OutputFlags::FF1
            | OutputFlags::NLDLY
            | OutputFlags::CRDLY
            | OutputFlags::TABDLY
            | OutputFlags::BSDLY
            | OutputFlags::VTDLY
            | OutputFlags::FFDLY);

        settings.control_flags |= ControlFlags::CS6
            | ControlFlags::CS7
            | ControlFlags::CS8
            | ControlFlags::CREAD
            | ControlFlags::CLOCAL
            | ControlFlags::CBAUDEX // NOTE also via cfsetspeed below
            | ControlFlags::CSIZE;
        settings.control_flags &= !(ControlFlags::HUPCL
            | ControlFlags::CS5
            | ControlFlags::CSTOPB
            | ControlFlags::PARENB
            | ControlFlags::PARODD
            | ControlFlags::CRTSCTS
            | ControlFlags::CBAUD // NOTE also set via cfsetspeed below?
            | ControlFlags::CMSPAR
            | ControlFlags::CIBAUD);

        settings.local_flags |= LocalFlags::ECHOKE
            | LocalFlags::ECHOE
            | LocalFlags::ECHOK
            | LocalFlags::ECHOCTL
            | LocalFlags::IEXTEN;
        settings.local_flags &= !(LocalFlags::ECHO
            | LocalFlags::ISIG
            | LocalFlags::ICANON
            | LocalFlags::ECHONL
            | LocalFlags::ECHOPRT
            | LocalFlags::EXTPROC
            | LocalFlags::TOSTOP
            | LocalFlags::FLUSHO
            | LocalFlags::PENDIN
            | LocalFlags::NOFLSH);

        termios::cfsetspeed(&mut settings, baud_rate).map_err(|e| {
            Error::General(format!(
                "Failed to configure device baud rate: cfsetspeed = {}",
                e
            ))
        })?;

        settings.control_chars[CC::VTIME as usize] = 2;
        settings.control_chars[CC::VMIN as usize] = 100;

        // Drain all output, flush all input, and apply settings.
        termios::tcsetattr(fd, SetArg::TCSAFLUSH, &settings).map_err(|e| {
            Error::General(format!(
                "Failed to apply terminal settings to device: tcsetattr = {}",
                e
            ))
        })?;

        let mut flags: libc::c_int = 0;
        ioctl::tiocmget(fd, &mut flags).map_err(|e| {
            Error::General(format!(
                "Failed to read modem bits of device: tiocmget = {}",
                e
            ))
        })?;
        flags |= libc::TIOCM_DTR | libc::TIOCM_RTS;
        ioctl::tiocmset(fd, &flags).map_err(|e| {
            Error::General(format!(
                "Failed to apply modem bits to device: tiocmset = {}",
                e
            ))
        })?;

        // Make the tty read-only.
        fcntl::fcntl(fd, FcntlArg::F_SETFL(OFlag::O_RDONLY)).map_err(|e| {
            Error::General(format!("Failed to make device read-only: fcntl = {}", e))
        })?;

        // Flush all pending I/O, just in case.
        ioctl::tcflsh(fd, libc::TCIOFLUSH).map_err(|e| {
            Error::General(format!("Failed to flush I/O of device: tcflsh = {}", e))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_to_baud_rate() {
        assert_eq!(
            Ok(BaudRate::B9600),
            BaudRate::try_from(ArbitraryBaudRate(9600))
        );
    }
}
