//! Internal HTTP/SSE dashboard API and static asset server.

include!("web_server.rs");
include!("web_simulation.rs");
include!("web_automation.rs");
include!("web_blocks.rs");
include!("web_events.rs");
#[cfg(test)]
mod tests {
    include!("web_tests.rs");
    include!("web_blocks_tests.rs");
}
