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
        let cpuid = x86::cpuid::CpuId::new();
        let family_id = cpuid.get_feature_info().map_or(0, |s| s.family_id());
        let model_id = cpuid.get_feature_info().map_or(0, |s| s.model_id());
        let stepping = cpuid.get_feature_info().map_or(0, |s| s.stepping_id());
        let nom_freq = cpuid
            .get_processor_frequency_info()
            .map_or(0, |s| s.processor_max_frequency() / 100);
        let tsc_ratio = cpuid
            .get_tsc_info()
            .map_or((0, 0), |s| (s.numerator(), s.denominator()));

        file.write_all(
            format!(
                "meta family {}
meta model {}
meta stepping {}
meta mtc_freq {}
meta nom_freq {}
meta tsc_ratio {} {}\n",
                family_id,
                model_id,
                stepping,
                self.settings.mtc_freq,
                nom_freq,
                tsc_ratio.1,
                tsc_ratio.0
            )
            .as_bytes(),
        )
        .unwrap();

        let pid = unsafe { libc::getpid() };
        Command::new("cp")
            .arg(format!("/proc/{}/maps", pid))
            .arg(format!("{}.ptmap", filename))
            .output()
            .expect("failed to execute process");
    }
}
