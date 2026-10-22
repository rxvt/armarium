//! Finds the connected Device: watches mounts for a Kindle, one at a time,
//! the first found winning. Development builds can stand a plain folder in
//! for one with `ARMARIUM_DEVICE=<folder>`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use armarium_core::device::Device;
use gtk::gio;
use gtk::prelude::*;

use crate::profile;

pub struct Connected {
    pub device: Device,
    /// The drive's name, e.g. "Kindle".
    pub name: String,
    /// `None` for a folder standing in for a Device.
    pub mount: Option<gio::Mount>,
}

pub struct DeviceWatch {
    /// Kept alive so its signals keep firing.
    _monitor: Option<gio::VolumeMonitor>,
}

impl DeviceWatch {
    /// Calls `changed` with the connected Device, if any, straight away and
    /// again whenever it is connected or disconnected.
    pub fn start(changed: impl Fn(Option<Connected>) + 'static) -> DeviceWatch {
        if profile::DEVEL
            && let Some(folder) = std::env::var_os("ARMARIUM_DEVICE").map(PathBuf::from)
        {
            match Device::at(&folder) {
                Some(device) => changed(Some(Connected {
                    device,
                    name: "Kindle".into(),
                    mount: None,
                })),
                None => eprintln!(
                    "ARMARIUM_DEVICE: {} needs documents/ and system/ folders",
                    folder.display()
                ),
            }
            return DeviceWatch { _monitor: None };
        }

        let monitor = gio::VolumeMonitor::get();
        // The mount the Device is on.
        let current: Rc<RefCell<Option<gio::Mount>>> = Rc::default();
        let connect = Rc::new({
            let current = current.clone();
            move |connected: Option<Connected>| {
                current.replace(connected.as_ref().and_then(|c| c.mount.clone()));
                changed(connected);
            }
        });
        connect(first_device(&monitor));
        monitor.connect_mount_added({
            let (current, connect) = (current.clone(), connect.clone());
            move |_, mount| {
                let free = current.borrow().is_none();
                if free && let Some(connected) = device_at(mount) {
                    connect(Some(connected));
                }
            }
        });
        monitor.connect_mount_removed(move |monitor, mount| {
            let ours = current.borrow().as_ref() == Some(mount);
            if ours {
                connect(first_device(monitor));
            }
        });
        DeviceWatch {
            _monitor: Some(monitor),
        }
    }
}

fn first_device(monitor: &gio::VolumeMonitor) -> Option<Connected> {
    monitor.mounts().iter().find_map(device_at)
}

fn device_at(mount: &gio::Mount) -> Option<Connected> {
    let device = Device::at(&mount.root().path()?)?;
    Some(Connected {
        device,
        name: mount.name().into(),
        mount: Some(mount.clone()),
    })
}
