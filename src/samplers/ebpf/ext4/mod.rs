// Copyright 2019 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

mod statistics;

use self::statistics::Statistic;
use super::map_from_table;
use crate::common::*;
use crate::config::*;
use crate::samplers::{Common, Sampler};

use bcc;
use bcc::core::BPF;
use failure::*;
use logger::*;
use metrics::*;
use time;

use std::sync::Arc;

pub struct Ext4 {
    bpf: BPF,
    common: Common,
}

impl Sampler for Ext4 {
    fn new(
        config: Arc<Config>,
        metrics: Metrics<AtomicU32>,
    ) -> Result<Option<Box<dyn Sampler>>, Error> {
        if config.ebpf().ext4() {
            debug!("initializing");
            // load the code and compile
            let code = include_str!("bpf.c").to_string();
            let addr = "0x".to_string() + &super::symbol_lookup("ext4_file_operations").unwrap();
            let code = code.replace("EXT4_FILE_OPERATIONS", &addr);
            let mut bpf = BPF::new(&code)?;

            // load + attach kprobes!

            let generic_file_read_iter_entry = bpf.load_kprobe("trace_read_entry")?;
            let ext4_file_write_iter_entry = bpf.load_kprobe("trace_entry")?;
            let ext4_file_open_entry = bpf.load_kprobe("trace_entry")?;
            let ext4_sync_file_entry = bpf.load_kprobe("trace_entry")?;
            let generic_file_read_iter_return = bpf.load_kprobe("trace_read_return")?;
            let ext4_file_write_iter_return = bpf.load_kprobe("trace_write_return")?;
            let ext4_file_open_return = bpf.load_kprobe("trace_open_return")?;
            let ext4_sync_file_return = bpf.load_kprobe("trace_fsync_return")?;

            bpf.attach_kprobe("generic_file_read_iter", generic_file_read_iter_entry)?;
            bpf.attach_kprobe("ext4_file_write_iter", ext4_file_write_iter_entry)?;
            bpf.attach_kprobe("ext4_file_open", ext4_file_open_entry)?;
            bpf.attach_kprobe("ext4_sync_file", ext4_sync_file_entry)?;
            bpf.attach_kretprobe("generic_file_read_iter", generic_file_read_iter_return)?;
            bpf.attach_kretprobe("ext4_file_write_iter", ext4_file_write_iter_return)?;
            bpf.attach_kretprobe("ext4_file_open", ext4_file_open_return)?;
            bpf.attach_kretprobe("ext4_sync_file", ext4_sync_file_return)?;

            Ok(Some(Box::new(Self {
                bpf,
                common: Common::new(config, metrics),
            }) as Box<dyn Sampler>))
        } else {
            Ok(None)
        }
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn name(&self) -> String {
        "ebpf::ext4".to_string()
    }

    fn sample(&mut self) -> Result<(), ()> {
        // gather current state
        trace!("sampling {}", self.name());
        let time = time::precise_time_ns();
        self.register();
        for statistic in &[
            Statistic::Fsync,
            Statistic::Open,
            Statistic::Read,
            Statistic::Write,
        ] {
            trace!("sampling {}", statistic);
            let mut table = self.bpf.table(&statistic.table_name());
            for (&value, &count) in &map_from_table(&mut table, MICROSECOND) {
                self.common
                    .record_distribution(statistic, time, value, count);
            }
        }
        Ok(())
    }

    fn interval(&self) -> usize {
        self.common()
            .config()
            .ebpf()
            .interval()
            .unwrap_or_else(|| self.common().config().interval())
    }

    fn register(&mut self) {
        if !self.common.initialized() {
            trace!("register {}", self.name());
            for statistic in &[
                Statistic::Fsync,
                Statistic::Open,
                Statistic::Read,
                Statistic::Write,
            ] {
                self.common
                    .register_distribution(statistic, SECOND, 2, PERCENTILES);
            }
            self.common.set_initialized(true);
        }
    }

    fn deregister(&mut self) {
        trace!("deregister {}", self.name());
        for statistic in &[
            Statistic::Fsync,
            Statistic::Open,
            Statistic::Read,
            Statistic::Write,
        ] {
            self.common.delete_channel(statistic);
        }
        self.common.set_initialized(false);
    }
}
