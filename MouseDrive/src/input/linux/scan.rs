use std::collections::HashMap;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use evdev::raw_stream::RawDevice;

use super::super::{Class, DeviceReport, Skip};
use super::classify::{DeviceCaps, classify, classify_among};
use super::device::{caps_of_device, open_node};

const DEV_INPUT: &str = "/dev/input";
const SYS_CLASS_INPUT: &str = "/sys/class/input";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NodeStamp {
    pub(super) rdev: u64,
    pub(super) ctime: i64,
    pub(super) ctime_nsec: i64,
}

impl NodeStamp {
    pub(super) fn of(node: &Path) -> Option<Self> {
        let meta = std::fs::metadata(node).ok()?;
        Some(Self {
            rdev: meta.rdev(),
            ctime: meta.ctime(),
            ctime_nsec: meta.ctime_nsec(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NodeEntry {
    pub(super) stamp: NodeStamp,
    pub(super) caps: Option<DeviceCaps>,
    pub(super) denied: bool,
}

#[derive(Default)]
pub(super) struct NodeCache(HashMap<PathBuf, NodeEntry>);

impl NodeCache {
    pub(super) fn refresh(&mut self, nodes: &[PathBuf]) {
        self.refresh_with(nodes, NodeStamp::of, sysfs_caps);
    }

    pub(super) fn refresh_with(
        &mut self,
        nodes: &[PathBuf],
        stamp_of: impl Fn(&Path) -> Option<NodeStamp>,
        read_caps: impl Fn(&Path) -> Option<DeviceCaps>,
    ) {
        self.0.retain(|node, _| nodes.contains(node));
        for node in nodes {
            self.refresh_node(node, stamp_of(node), &read_caps);
        }
    }

    pub(super) fn refresh_node(
        &mut self,
        node: &Path,
        stamp: Option<NodeStamp>,
        read_caps: impl Fn(&Path) -> Option<DeviceCaps>,
    ) {
        let Some(stamp) = stamp else {
            self.0.remove(node);
            return;
        };
        if self.0.get(node).is_some_and(|e| e.stamp == stamp) {
            return;
        }
        let entry = NodeEntry {
            stamp,
            caps: read_caps(node),
            denied: false,
        };
        self.0.insert(node.to_path_buf(), entry);
    }

    pub(super) fn get_mut(&mut self, node: &Path) -> Option<&mut NodeEntry> {
        self.0.get_mut(node)
    }

    pub(super) fn caps(&self) -> Vec<DeviceCaps> {
        self.0.values().filter_map(|e| e.caps.clone()).collect()
    }

    pub(super) fn verdict(&self, node: &Path, all: &[DeviceCaps]) -> Option<Result<Class, Skip>> {
        let caps = self.0.get(node)?.caps.as_ref()?;
        Some(classify_among(caps, all))
    }
}

pub(super) struct Opened {
    pub(super) dev: RawDevice,
    pub(super) caps: DeviceCaps,
    pub(super) class: Class,
}

pub(super) enum Verdict {
    Open(Box<Opened>),
    Skip(Option<Class>, Skip),
}

pub(super) fn evaluate(node: &Path, entry: &mut NodeEntry, all: &[DeviceCaps]) -> Verdict {
    let planned = entry.caps.as_ref().map(|caps| classify_among(caps, all));
    if let Some(Err(skip)) = planned {
        return Verdict::Skip(own_class(entry.caps.as_ref()), skip);
    }
    let class = planned.and_then(Result::ok);
    if entry.denied {
        return Verdict::Skip(class, Skip::PermissionDenied);
    }
    match open_node(node) {
        Ok(dev) => {
            let caps = entry
                .caps
                .get_or_insert_with(|| caps_of_device(&dev))
                .clone();
            match classify_among(&caps, all) {
                Ok(class) => Verdict::Open(Box::new(Opened { dev, caps, class })),
                Err(skip) => Verdict::Skip(own_class(Some(&caps)), skip),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            entry.denied = true;
            Verdict::Skip(class, Skip::PermissionDenied)
        }
        Err(e) => Verdict::Skip(class, Skip::OpenFailed(e.kind())),
    }
}

fn own_class(caps: Option<&DeviceCaps>) -> Option<Class> {
    caps.and_then(|caps| classify(caps).ok())
}

pub(super) fn device_report(
    node: &Path,
    caps: Option<&DeviceCaps>,
    class: Option<Class>,
    skip: Option<Skip>,
) -> DeviceReport {
    let caps = caps.cloned().unwrap_or_default();
    DeviceReport {
        node: node.display().to_string(),
        name: caps.name,
        bus: caps.bus,
        vendor: caps.vendor,
        product: caps.product,
        uniq: caps.uniq,
        class,
        skip,
    }
}

pub(super) fn list_devices() -> Vec<DeviceReport> {
    let nodes = event_nodes();
    let mut cache = NodeCache::default();
    cache.refresh(&nodes);
    let all = cache.caps();
    nodes
        .iter()
        .filter_map(|node| {
            let entry = cache.get_mut(node)?;
            Some(match evaluate(node, entry, &all) {
                Verdict::Open(opened) => {
                    device_report(node, Some(&opened.caps), Some(opened.class), None)
                }
                Verdict::Skip(class, skip) => {
                    device_report(node, entry.caps.as_ref(), class, Some(skip))
                }
            })
        })
        .collect()
}

pub(super) fn event_nodes() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(DEV_INPUT) else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    nodes.sort_by_key(|node| node_order(node));
    nodes
}

fn node_order(node: &Path) -> (u32, PathBuf) {
    let number = node
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("event"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX);
    (number, node.to_path_buf())
}

fn sysfs_caps(node: &Path) -> Option<DeviceCaps> {
    let event = node.file_name()?;
    DeviceCaps::from_sysfs(&Path::new(SYS_CLASS_INPUT).join(event).join("device"))
}
