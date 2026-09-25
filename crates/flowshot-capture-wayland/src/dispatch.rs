//! `wayland-client` [`Dispatch`] implementations for the capture session.
//!
//! Hand-rolled registry binds in the libwayshot/`grim` single-purpose
//! client style: every event only fills plain data in [`CaptureState`]; no
//! request is ever issued from inside a callback except the binds driven by
//! registry `global` events.

use flowshot_core::geometry::Transform;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

use crate::globals::Global;
use crate::session::{CaptureState, OutputKey};
use crate::transform::transform_from_wl_output;

impl Dispatch<WlRegistry, ()> for CaptureState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => state.record_global(registry, Global::new(name, interface, version), qh),
            wl_registry::Event::GlobalRemove { name } => state.remove_global(name),
            // The generated event enum is #[non_exhaustive]; wl_registry has
            // no other events and unknown ones cannot survive parsing.
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, OutputKey> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &WlOutput,
        event: wl_output::Event,
        data: &OutputKey,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // A removed output can still have queued events; dropping them is
        // the correct handling.
        let Some(output) = state.outputs.get_mut(&data.0) else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x,
                y,
                make,
                model,
                transform,
                ..
            } => {
                output.data.geometry_position = Some((x, y));
                output.data.make = make;
                output.data.model = model;
                output.data.transform = match transform.into_result() {
                    Ok(known) => transform_from_wl_output(known),
                    Err(unknown) => {
                        tracing::warn!(
                            ?unknown,
                            "unknown wl_output transform value; assuming normal orientation"
                        );
                        Transform::Normal
                    }
                };
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                let is_current = flags
                    .into_result()
                    .is_ok_and(|flags| flags.contains(wl_output::Mode::Current));
                if is_current {
                    output.data.current_mode = Some((width, height));
                }
            }
            wl_output::Event::Scale { factor } => output.data.scale = factor,
            wl_output::Event::Name { name } => output.data.wl_name = Some(name),
            wl_output::Event::Description { description } => {
                output.data.wl_description = Some(description);
            }
            // Done marks the end of the initial burst; no state to update.
            // The generated event enum is #[non_exhaustive].
            wl_output::Event::Done | _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &ZxdgOutputManagerV1,
        _event: <ZxdgOutputManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // zxdg_output_manager_v1 has no events.
    }
}

impl Dispatch<ZxdgOutputV1, OutputKey> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ZxdgOutputV1,
        event: <ZxdgOutputV1 as wayland_client::Proxy>::Event,
        data: &OutputKey,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(&data.0) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                output.data.logical_position = Some((x, y));
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.data.logical_size = Some((width, height));
            }
            zxdg_output_v1::Event::Name { name } => output.data.xdg_name = Some(name),
            zxdg_output_v1::Event::Description { description } => {
                output.data.xdg_description = Some(description);
            }
            // Done ends the object's initial burst (and is deprecated in
            // zxdg_output_v1 version 3); the generated enum is
            // #[non_exhaustive].
            zxdg_output_v1::Event::Done | _ => {}
        }
    }
}
