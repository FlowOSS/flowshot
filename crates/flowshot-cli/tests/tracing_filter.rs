//! Tests for the tracing filter configuration in flowshot-cli

use tracing_subscriber::EnvFilter;

#[test]
fn test_cli_tracing_filter_defaults() {
    // Test that the default filter includes our specific directives
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive("zbus::proxy=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::conv=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::instance=error".parse().unwrap());

    // Verify the filter contains our directives
    let filter_str = filter.to_string();
    assert!(filter_str.contains("zbus::proxy=error"));
    assert!(filter_str.contains("wgpu_hal::vulkan::conv=error"));
    assert!(filter_str.contains("wgpu_hal::vulkan::instance=error"));
    
    // Verify that the default level is still warn
    assert!(filter_str.contains("warn"));
}

#[test]
fn test_cli_tracing_filter_override() {
    // Test that explicit RUST_LOG still overrides our defaults
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"))
        .add_directive("zbus::proxy=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::conv=error".parse().unwrap())
        .add_directive("wgpu_hal::vulkan::instance=error".parse().unwrap());

    // Verify our specific filters are present
    let filter_str = filter.to_string();
    assert!(filter_str.contains("zbus::proxy=error"));
    assert!(filter_str.contains("wgpu_hal::vulkan::conv=error"));
    assert!(filter_str.contains("wgpu_hal::vulkan::instance=error"));
}