//! Find the mouse's vendor hidraw node and send feature reports to it.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::protocol::{get_request, Report, Step, REPORT_ID};

const VENDOR_ID: u32 = 0x30FA;
const PRODUCT_IDS: [u32; 3] = [0x1040, 0x1140, 0x1440];
const SYSFS: &str = "/sys/class/hidraw";

/// Usage Page (0xFF01) followed (in the same collection) by Report ID 7.
const VENDOR_PAGE: [u8; 3] = [0x06, 0x01, 0xFF];
const REPORT_ID_7: [u8; 2] = [0x85, REPORT_ID];

const fn hidiocsfeature(len: usize) -> libc::c_ulong {
    ((3 << 30) | (len << 16) | ((b'H' as usize) << 8) | 0x06) as libc::c_ulong
}

const fn hidiocgfeature(len: usize) -> libc::c_ulong {
    ((3 << 30) | (len << 16) | ((b'H' as usize) << 8) | 0x07) as libc::c_ulong
}

fn hid_id(node: &Path) -> Option<(u32, u32)> {
    let uevent = fs::read_to_string(node.join("device/uevent")).ok()?;
    let id = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
    let mut parts = id.split(':').skip(1);
    let vid = u32::from_str_radix(parts.next()?, 16).ok()?;
    let pid = u32::from_str_radix(parts.next()?, 16).ok()?;
    Some((vid, pid))
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// /dev paths of the vendor interface of every connected IMG-355GX.
pub fn find() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = match fs::read_dir(SYSFS) {
        Ok(dir) => dir.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => return Vec::new(),
    };
    nodes.sort();
    nodes
        .into_iter()
        .filter(|node| {
            let Some((vid, pid)) = hid_id(node) else { return false };
            if vid != VENDOR_ID || !PRODUCT_IDS.contains(&pid) {
                return false;
            }
            let Ok(desc) = fs::read(node.join("device/report_descriptor")) else { return false };
            find_subslice(&desc, &VENDOR_PAGE).is_some_and(|i| find_subslice(&desc[i..], &REPORT_ID_7).is_some())
        })
        .map(|node| Path::new("/dev").join(node.file_name().unwrap()))
        .collect()
}

pub fn open(path: Option<&Path>, write: bool) -> Result<File> {
    let path = match path {
        Some(p) => p.to_path_buf(),
        None => match find().into_iter().next() {
            Some(p) => p,
            None => bail!("IMG-355GX mouse not found (30fa:1040). Is it plugged in?"),
        },
    };
    match OpenOptions::new().read(true).write(write).open(&path) {
        Ok(f) => Ok(f),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => bail!(
            "no permission to open {}: install contrib/70-img355-mouse.rules into /etc/udev/rules.d and re-plug the mouse",
            path.display()
        ),
        Err(e) => Err(e).with_context(|| format!("opening {}", path.display())),
    }
}

pub struct Mouse {
    file: File,
}

impl Mouse {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        Ok(Mouse { file: open(path, true)? })
    }

    pub fn send(&self, report: &Report) -> Result<()> {
        let mut buf = *report;
        // SAFETY: HIDIOCSFEATURE reads exactly `buf.len()` bytes from the pointer.
        let rc = unsafe { libc::ioctl(self.file.as_raw_fd(), hidiocsfeature(buf.len()) as _, buf.as_mut_ptr()) };
        if rc < 0 {
            bail!("sending report failed: {}", io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn get(&self) -> Result<Report> {
        let mut buf = get_request();
        // SAFETY: HIDIOCGFEATURE writes at most `buf.len()` bytes into the buffer.
        let rc = unsafe { libc::ioctl(self.file.as_raw_fd(), hidiocgfeature(buf.len()) as _, buf.as_mut_ptr()) };
        if rc < 0 {
            bail!("reading status report failed: {}", io::Error::last_os_error());
        }
        Ok(buf)
    }

    /// Runs an Apply sequence, with reports about 1 ms apart.
    pub fn run(&self, steps: &[Step]) -> Result<()> {
        for step in steps {
            match step {
                Step::Set(r) => self.send(r)?,
                Step::Get => {
                    self.get()?;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Ok(())
    }
}
