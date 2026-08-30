use crate::*;
use logiksmith_core::{
    BlockConfig as CoreBlockConfig, BlockId, BlockSchedule as CoreBlockSchedule, Coordinates, Dpt,
    Endpoint, EndpointDirection, EndpointName, EngineConfig, LocalTime,
    RuntimeConfig as CoreRuntimeConfig, ScheduleName, ScheduleRule,
    SignalBinding as CoreSignalBinding, SignalConfig as CoreSignalConfig, SignalName,
    SiteTimeConfig, SolarAnchor, TimeZoneId, Weekday, WeekdaySet,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
};
use tracing_subscriber::filter::LevelFilter;
include!("configuration_fields.rs");
include!("automation_builder.rs");
