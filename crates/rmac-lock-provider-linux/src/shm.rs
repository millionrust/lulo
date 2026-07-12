//! Immutable Linux `memfd` backing for one painted `wl_shm` buffer.

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write as _};
use std::os::fd::{AsFd, BorrowedFd};

use rustix::fs::{fcntl_add_seals, ftruncate, memfd_create, MemfdFlags, SealFlags};

use crate::paint::{paint_lock_frame, LockPalette};
use crate::surface::{BufferId, BufferLayout, RenderPlan};

pub struct ShmFrame {
    buffer: BufferId,
    layout: BufferLayout,
    file: File,
}

impl ShmFrame {
    pub fn paint(plan: &RenderPlan, palette: LockPalette) -> Result<Self, Error> {
        let layout = plan.layout();
        let fd = memfd_create(
            "rmac-lock-frame",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::NOEXEC_SEAL,
        )
        .map_err(Error::Create)?;
        ftruncate(&fd, layout.byte_len()).map_err(Error::Resize)?;
        let file = File::from(fd);
        {
            let mut writer = BufWriter::with_capacity(64 * 1024, &file);
            paint_lock_frame(&mut writer, layout, palette).map_err(Error::Paint)?;
            writer.flush().map_err(Error::Paint)?;
        }
        fcntl_add_seals(
            &file,
            SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
        )
        .map_err(Error::Seal)?;

        Ok(Self {
            buffer: plan.buffer(),
            layout,
            file,
        })
    }

    pub fn buffer(&self) -> BufferId {
        self.buffer
    }

    pub fn layout(&self) -> BufferLayout {
        self.layout
    }

    pub fn pool_size(&self) -> i32 {
        self.layout.byte_len() as i32
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl fmt::Debug for ShmFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShmFrame")
            .field("buffer", &"<redacted>")
            .field("layout", &self.layout)
            .finish()
    }
}

#[derive(Debug)]
pub enum Error {
    Create(rustix::io::Errno),
    Resize(rustix::io::Errno),
    Paint(io::Error),
    Seal(rustix::io::Errno),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create(_) => formatter.write_str("cannot create lock-frame shared memory"),
            Self::Resize(_) => formatter.write_str("cannot size lock-frame shared memory"),
            Self::Paint(_) => formatter.write_str("cannot paint lock-frame shared memory"),
            Self::Seal(_) => formatter.write_str("cannot seal lock-frame shared memory"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Create(error) | Self::Resize(error) | Self::Seal(error) => Some(error),
            Self::Paint(error) => Some(error),
        }
    }
}
