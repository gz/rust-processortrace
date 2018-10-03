use std::fs::File;
use std::io::prelude::*;
use std::process::Command;

use libc;

use alloc::vec::Vec;
use {PTInfo, TraceControllerSettings, TraceDumpControl};

pub struct TraceDump {
    pub data: Vec<u8>,
    settings: TraceControllerSettings,
}

impl TraceDump {
    pub fn new(raw_data: &[u8], settings: TraceControllerSettings) -> TraceDump {
        let mut data = Vec::with_capacity(raw_data.len());
        data.extend_from_slice(raw_data);
        TraceDump {
            data: data,
            settings: settings,
        }
    }
}

impl TraceDumpControl for TraceDump {
    fn save(&self, filename: &str) {
        let mut file = File::create(format!("{}.ptdump", filename)).unwrap();
        file.write_all(self.data.as_slice() as &[u8]).unwrap();

        let mut file = File::create(format!("{}.ptsettings", filename)).unwrap();
        file.write_all(format!("{:?}\n", self.settings).as_bytes())
            .unwrap();

        let mut file = File::create(format!("{}.ptinfo", filename)).unwrap();
        file.write_all(format!("{:?}\n", PTInfo::new()).as_bytes())
            .unwrap();

        let pid = unsafe { libc::getpid() };
        Command::new("cp")
            .arg(format!("/proc/{}/maps", pid))
            .arg(format!("{}.ptmap", filename))
            .output()
            .expect("failed to execute process");
    }
}
