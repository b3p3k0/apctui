// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed schema for the apcupsd.conf directives apctui edits.
//!
//! We deliberately cover the directives a user manages day to day (identity,
//! connection, power policy, NIS, logging) and leave the apctest/EEPROM
//! directives out of the structured editor — they're presented as raw
//! passthrough so we never silently drop them on save.

/// Kind of value a directive accepts, driving both the editor widget and
/// validation.
#[derive(Debug, Clone)]
pub enum Kind {
    /// Free text (may be empty), e.g. UPSNAME, DEVICE.
    Text,
    /// One of a fixed set (rendered as a cycler/dropdown).
    Enum(&'static [&'static str]),
    /// Integer within [min, max]. `sentinel` documents a special value.
    Int { min: i64, max: i64, #[allow(dead_code)] sentinel: Option<(i64, &'static str)> },
    /// on/off toggle.
    Bool,
}

#[derive(Debug, Clone)]
pub struct Directive {
    pub key: &'static str,
    pub kind: Kind,
    pub group: Group,
    /// One-line help shown in the editor.
    pub help: &'static str,
    /// True for directives that, if present, most users should keep.
    pub common: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Identity,
    Connection,
    PowerPolicy,
    Nis,
    Logging,
    Sharing,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Identity => "Identity",
            Group::Connection => "Connection",
            Group::PowerPolicy => "Power policy",
            Group::Nis => "Network (NIS)",
            Group::Logging => "Logging",
            Group::Sharing => "Sharing",
        }
    }

    pub fn order() -> &'static [Group] {
        &[
            Group::Identity,
            Group::Connection,
            Group::PowerPolicy,
            Group::Nis,
            Group::Logging,
            Group::Sharing,
        ]
    }
}

pub const UPSTYPES: &[&str] =
    &["dumb", "apcsmart", "net", "usb", "snmp", "pcnet", "modbus", "test"];
pub const UPSCABLES: &[&str] = &[
    "simple", "smart", "ether", "usb", "940-0119A", "940-0127A", "940-0128A",
    "940-0020B", "940-0020C", "940-0023A", "940-0024B", "940-0024C", "940-1524C",
    "940-0024G", "940-0095A", "940-0095B", "940-0095C", "940-0625A", "MAM-04-02-2000",
];
pub const NOLOGON: &[&str] = &["disable", "timeout", "percent", "minutes", "always"];
pub const UPSCLASS: &[&str] = &["standalone", "shareslave", "sharemaster"];
pub const UPSMODE: &[&str] = &["disable", "share"];

/// The directive catalog, in editor display order within each group.
pub fn catalog() -> &'static [Directive] {
    use Group::*;
    use Kind::*;
    &[
        Directive { key: "UPSNAME", kind: Text, group: Identity, common: true,
            help: "Name for this UPS in logs and status (<=8 chars for EEPROM)." },
        Directive { key: "UPSCABLE", kind: Enum(UPSCABLES), group: Connection, common: true,
            help: "Cable connecting UPS to host. 'usb' for USB units." },
        Directive { key: "UPSTYPE", kind: Enum(UPSTYPES), group: Connection, common: true,
            help: "Driver type. 'usb' for USB, 'net' for a network client." },
        Directive { key: "DEVICE", kind: Text, group: Connection, common: true,
            help: "Device path. Leave BLANK for USB; host:port for net." },
        Directive { key: "POLLTIME", kind: Int { min: 1, max: 600, sentinel: None },
            group: Connection, common: false,
            help: "Seconds between status polls (default 60; drops to 1 on battery)." },
        Directive { key: "ONBATTERYDELAY", kind: Int { min: 0, max: 600, sentinel: None },
            group: PowerPolicy, common: true,
            help: "Seconds after power loss before the onbattery event fires." },
        Directive { key: "BATTERYLEVEL",
            kind: Int { min: -1, max: 100, sentinel: Some((-1, "disabled")) },
            group: PowerPolicy, common: true,
            help: "Shut down when battery % falls to/below this (-1 disables)." },
        Directive { key: "MINUTES", kind: Int { min: -1, max: 1440, sentinel: Some((-1, "disabled")) },
            group: PowerPolicy, common: true,
            help: "Shut down when estimated runtime (min) falls to/below this." },
        Directive { key: "TIMEOUT", kind: Int { min: 0, max: 86400, sentinel: Some((0, "off")) },
            group: PowerPolicy, common: true,
            help: "Force shutdown after N s on battery (0 = use battery/runtime)." },
        Directive { key: "ANNOY", kind: Int { min: 0, max: 3600, sentinel: None },
            group: PowerPolicy, common: false,
            help: "Seconds between 'log off' broadcasts while on battery." },
        Directive { key: "ANNOYDELAY", kind: Int { min: 0, max: 3600, sentinel: None },
            group: PowerPolicy, common: false,
            help: "Delay before the first log-off broadcast." },
        Directive { key: "NOLOGON", kind: Enum(NOLOGON), group: PowerPolicy, common: false,
            help: "When to block new logins during a power failure." },
        Directive { key: "KILLDELAY", kind: Int { min: 0, max: 3600, sentinel: Some((0, "disabled")) },
            group: PowerPolicy, common: false,
            help: "Seconds before apcupsd kills UPS power after shutdown (0 off)." },
        Directive { key: "NETSERVER", kind: Bool, group: Nis, common: true,
            help: "Serve status/events over the network (NIS)." },
        Directive { key: "NISIP", kind: Text, group: Nis, common: true,
            help: "Interface to bind. 0.0.0.0 = all; 127.0.0.1 = local only." },
        Directive { key: "NISPORT", kind: Int { min: 1, max: 65535, sentinel: None },
            group: Nis, common: true,
            help: "NIS TCP port. Must be unique per instance (3551, 3552, ...)." },
        Directive { key: "EVENTSFILE", kind: Text, group: Logging, common: false,
            help: "Path to the events log (unique per instance)." },
        Directive { key: "EVENTSFILEMAX", kind: Int { min: 0, max: 1000, sentinel: Some((0, "unlimited")) },
            group: Logging, common: false,
            help: "Max kB of the events file before rotation." },
        Directive { key: "STATTIME", kind: Int { min: 0, max: 3600, sentinel: Some((0, "off")) },
            group: Logging, common: false,
            help: "Seconds between status-file writes (0 disables)." },
        Directive { key: "STATFILE", kind: Text, group: Logging, common: false,
            help: "Path to the status file (unique per instance)." },
        Directive { key: "LOGSTATS", kind: Bool, group: Logging, common: false,
            help: "Verbose stats logging (high volume; needs a named pipe)." },
        Directive { key: "DATATIME", kind: Int { min: 0, max: 3600, sentinel: Some((0, "off")) },
            group: Logging, common: false,
            help: "Seconds between PowerChute-style data log writes." },
        Directive { key: "LOCKFILE", kind: Text, group: Connection, common: false,
            help: "Serial/USB port lock directory (unique per instance)." },
        Directive { key: "SCRIPTDIR", kind: Text, group: Connection, common: false,
            help: "Directory holding apccontrol and event scripts." },
        Directive { key: "PWRFAILDIR", kind: Text, group: Connection, common: false,
            help: "Where the powerfail flag file is written." },
        Directive { key: "NOLOGINDIR", kind: Text, group: Connection, common: false,
            help: "Where the nologin file is written." },
        Directive { key: "UPSCLASS", kind: Enum(UPSCLASS), group: Sharing, common: false,
            help: "standalone for the normal one-host-per-UPS case." },
        Directive { key: "UPSMODE", kind: Enum(UPSMODE), group: Sharing, common: false,
            help: "disable unless using a Share-UPS expander." },
    ]
}

pub fn lookup(key: &str) -> Option<&'static Directive> {
    catalog().iter().find(|d| d.key.eq_ignore_ascii_case(key))
}
