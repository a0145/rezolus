// Copyright 2019 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

mod device;
mod entry;
mod statistics;

pub use self::device::Device;
pub use self::entry::Entry;
pub(crate) use self::statistics::Statistic;

use crate::common::*;
use crate::config::*;
use crate::samplers::{Common, Sampler};

use failure::Error;
use logger::*;
use metrics::*;
use regex::Regex;
use time;
use walkdir::WalkDir;

use std::collections::HashMap;
use std::sync::Arc;

const REFRESH: u64 = 60_000_000_000;

pub struct Disk {
    common: Common,
    devices: Vec<Device>,
    last_refreshed: u64,
}

impl Disk {
    /// send deltas to the stats library
    fn record(&self, time: u64, reading: Entry) {
        self.common
            .record_counter(&Statistic::BandwidthDiscard, time, reading.discard_bytes());
        self.common
            .record_counter(&Statistic::BandwidthRead, time, reading.read_bytes());
        self.common
            .record_counter(&Statistic::BandwidthWrite, time, reading.write_bytes());
        self.common
            .record_counter(&Statistic::OperationsDiscard, time, reading.discard_ops());
        self.common
            .record_counter(&Statistic::OperationsRead, time, reading.read_ops());
        self.common
            .record_counter(&Statistic::OperationsWrite, time, reading.write_ops());
    }

    /// identifies the set of all primary block `Device`s on the host
    fn get_devices(&self) -> Vec<Device> {
        let sd = Regex::new(r"^[a-z]+$").unwrap();
        let nvme = Regex::new(r"^nvme[0-9]+n[0-9]+$").unwrap();
        let mut result = Vec::new();
        for entry in WalkDir::new("/sys/class/block/").max_depth(1) {
            if let Ok(entry) = entry {
                if let Some(s) = entry.file_name().to_str() {
                    if s != "block" && (sd.is_match(s) || nvme.is_match(s)) {
                        trace!("Found block dev: {}", s);
                        result.push(Device::new(Some(s.to_owned())));
                    } else {
                        trace!("Ignore block dev: {}", s);
                    }
                }
            }
        }
        result
    }
}

impl Sampler for Disk {
    fn new(
        config: Arc<Config>,
        metrics: Metrics<AtomicU32>,
    ) -> Result<Option<Box<dyn Sampler>>, Error> {
        if config.disk().enabled() {
            Ok(Some(Box::new(Self {
                common: Common::new(config, metrics),
                devices: Vec::new(),
                last_refreshed: 0,
            }) as Box<dyn Sampler>))
        } else {
            Ok(None)
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn name(&self) -> String {
        "disk".to_string()
    }

    fn sample(&mut self) -> Result<(), ()> {
        trace!("sample {}", self.name());
        let time = time::precise_time_ns();
        let mut current = HashMap::new();
        if (time - self.last_refreshed) >= REFRESH {
            self.devices = self.get_devices();
            self.last_refreshed = time;
        }
        for device in self.devices.clone() {
            let entry = Entry::for_device(&device);
            current.insert(device, entry);
        }
        self.register();
        self.record(time, current.values().sum());
        Ok(())
    }

    fn interval(&self) -> usize {
        self.common()
            .config()
            .disk()
            .interval()
            .unwrap_or_else(|| self.common().config().interval())
    }

    fn register(&mut self) {
        if !self.common.initialized() {
            trace!("register {}", self.name());
            self.devices = self.get_devices();
            self.last_refreshed = time::precise_time_ns();
            for statistic in &[
                Statistic::BandwidthDiscard,
                Statistic::BandwidthRead,
                Statistic::BandwidthWrite,
            ] {
                self.common
                    .register_counter(statistic, TRILLION, 3, PERCENTILES);
            }
            for statistic in &[
                Statistic::OperationsDiscard,
                Statistic::OperationsRead,
                Statistic::OperationsWrite,
            ] {
                self.common
                    .register_counter(statistic, BILLION, 3, PERCENTILES);
            }
            self.common.set_initialized(true);
        }
    }

    fn deregister(&mut self) {
        trace!("deregister {}", self.name());
        for statistic in &[
            Statistic::BandwidthDiscard,
            Statistic::BandwidthRead,
            Statistic::BandwidthWrite,
            Statistic::OperationsDiscard,
            Statistic::OperationsRead,
            Statistic::OperationsWrite,
        ] {
            self.common.delete_channel(statistic);
        }
        self.common.set_initialized(false);
    }
}
