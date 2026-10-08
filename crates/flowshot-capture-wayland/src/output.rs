//! Output tracking: `wl_output` + `zxdg_output` event data mapped onto the
//! shared [`OutputInfo`] geometry.
//!
//! `zxdg_output_v1` is the preferred source for logical geometry (position
//! and size in the global compositor space, connector name, description);
//! `wl_output` alone only gives integer scale, a pre-transform mode size and
//! a deprecated physical position, which serve as the documented fallback
//! for compositors without `xdg-output`.

use flowshot_core::geometry::{
    GeometryError, Logical, LogicalRect, LogicalSize, OutputInfo, PhysicalPx, PhysicalSize,
    ToLogical, ToPhysical, Transform,
};

/// Everything the compositor reported about one output, as plain data.
///
/// Fields mirror the protocol events that fill them (`wl_output`
/// geometry/mode/scale/name/description and `zxdg_output_v1`
/// logical-position/logical-size/name/description); `None` means the event
/// has not arrived. Kept separate from the wayland proxies so the mapping to
/// [`OutputInfo`] is unit-testable without a live socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputData {
    /// The `wl_registry` name the output was bound under.
    pub registry_name: u32,
    /// Manufacturer from `wl_output.geometry` (e.g. `DEL`).
    pub make: String,
    /// Model from `wl_output.geometry` (e.g. `DELL U2723QE`).
    pub model: String,
    /// Transform from `wl_output.geometry`.
    pub transform: Transform,
    /// Integer scale from `wl_output.scale` (protocol default: 1).
    pub scale: i32,
    /// Output position from `wl_output.geometry` (deprecated by the protocol
    /// in favor of `zxdg_output_v1.logical_position`; physical-pixel space
    /// and only correct at integer scale - fallback use only).
    pub geometry_position: Option<(i32, i32)>,
    /// Size of the mode flagged current, in physical pixels before
    /// `transform`, from `wl_output.mode`.
    pub current_mode: Option<(i32, i32)>,
    /// Connector name from `wl_output.name` (version 4+).
    pub wl_name: Option<String>,
    /// Human-readable description from `wl_output.description` (version 4+).
    pub wl_description: Option<String>,
    /// Position in global logical space from `zxdg_output_v1`.
    pub logical_position: Option<(i32, i32)>,
    /// Size in global logical space from `zxdg_output_v1`.
    pub logical_size: Option<(i32, i32)>,
    /// Connector name from `zxdg_output_v1` (version 2+), e.g. `DP-3`.
    pub xdg_name: Option<String>,
    /// Human-readable description from `zxdg_output_v1` (version 2+).
    pub xdg_description: Option<String>,
}

impl OutputData {
    /// Creates an empty record for a freshly bound output; the protocol
    /// default scale is 1 and the default transform is normal.
    pub(crate) fn new(registry_name: u32) -> Self {
        Self {
            registry_name,
            make: String::new(),
            model: String::new(),
            transform: Transform::Normal,
            scale: 1,
            geometry_position: None,
            current_mode: None,
            wl_name: None,
            wl_description: None,
            logical_position: None,
            logical_size: None,
            xdg_name: None,
            xdg_description: None,
        }
    }

    /// Assembles the shared [`OutputInfo`], preferring `zxdg_output_v1`
    /// logical geometry and falling back to `wl_output` data.
    ///
    /// A non-positive reported scale is normalized to 1 (the protocol
    /// default) instead of failing the output.
    ///
    /// # Errors
    ///
    /// Returns the [`GeometryError`] from [`OutputInfo::new`] when the
    /// compositor reported contradictory numbers (e.g. a negative mode
    /// size); the caller skips such an output with a warning.
    pub(crate) fn to_output_info(&self) -> Result<OutputInfo, GeometryError> {
        let scale = if self.scale > 0 {
            f64::from(self.scale)
        } else {
            1.0
        };
        let physical_size = self.physical_size(scale);
        let logical_rect = self.logical_rect(scale, physical_size);
        let connector = self.connector();
        let name = self.human_name(&connector);
        OutputInfo::new(
            connector,
            name,
            logical_rect,
            physical_size,
            scale,
            self.transform,
        )
    }

    /// Native buffer size before `transform`: the current mode when
    /// reported, otherwise derived from the logical size (inverse
    /// transform, inverse scale), otherwise degenerate zero.
    fn physical_size(&self, scale: f64) -> PhysicalSize {
        if let Some((width, height)) = self.current_mode {
            return PhysicalSize::new(PhysicalPx(width), PhysicalPx(height));
        }
        if let Some((width, height)) = self.logical_size {
            let buffer = LogicalSize::new(Logical(f64::from(width)), Logical(f64::from(height)))
                .to_physical(scale);
            return self.transform.inverse().apply_to_size(buffer);
        }
        PhysicalSize::new(PhysicalPx(0), PhysicalPx(0))
    }

    /// Global logical rect: `zxdg_output_v1` position and size when
    /// reported, otherwise the `wl_output` geometry position with the size
    /// implied by buffer and scale.
    fn logical_rect(&self, scale: f64, physical_size: PhysicalSize) -> LogicalRect {
        let (x, y) = self
            .logical_position
            .or(self.geometry_position)
            .unwrap_or((0, 0));
        let size = match self.logical_size {
            Some((width, height)) => {
                LogicalSize::new(Logical(f64::from(width)), Logical(f64::from(height)))
            }
            None => self
                .transform
                .apply_to_size(physical_size)
                .to_logical(scale),
        };
        LogicalRect::new(
            Logical(f64::from(x)),
            Logical(f64::from(y)),
            size.width,
            size.height,
        )
    }

    /// Connector identity: `zxdg_output_v1.name`, then `wl_output.name`
    /// (version 4+), then make and model, then the registry name.
    fn connector(&self) -> String {
        if let Some(name) = &self.xdg_name {
            return name.clone();
        }
        if let Some(name) = &self.wl_name {
            return name.clone();
        }
        let make_model = format!("{} {}", self.make, self.model);
        let trimmed = make_model.trim();
        if trimmed.is_empty() {
            return format!("output-{}", self.registry_name);
        }
        trimmed.to_owned()
    }

    /// Human-readable name: `zxdg_output_v1.description`, then
    /// `wl_output.description`, then the model, then the connector.
    fn human_name(&self, connector: &str) -> String {
        if let Some(description) = &self.xdg_description {
            return description.clone();
        }
        if let Some(description) = &self.wl_description {
            return description.clone();
        }
        if !self.model.is_empty() {
            return self.model.clone();
        }
        connector.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // Fixtures use integral-valued f64 (the integral-valued-f64 fixture convention), so
    // exact comparison is the intended assertion.
    #![allow(clippy::float_cmp)]

    use super::*;

    /// A fully reported output: the `zxdg_output` values win everywhere.
    fn full_output() -> OutputData {
        OutputData {
            registry_name: 7,
            make: "DEL".to_owned(),
            model: "DELL U2723QE".to_owned(),
            transform: Transform::Normal,
            scale: 2,
            geometry_position: Some((0, 0)),
            current_mode: Some((3840, 2160)),
            wl_name: Some("DP-3".to_owned()),
            wl_description: Some("wl description".to_owned()),
            logical_position: Some((1920, 0)),
            logical_size: Some((1920, 1080)),
            xdg_name: Some("DP-3".to_owned()),
            xdg_description: Some("Dell Inc. DELL U2723QE".to_owned()),
        }
    }

    #[test]
    fn full_report_maps_xdg_geometry_and_identity() {
        let info = full_output().to_output_info().unwrap();
        assert_eq!(info.connector, "DP-3");
        assert_eq!(info.name, "Dell Inc. DELL U2723QE");
        assert_eq!(info.logical_rect.x.0, 1920.0);
        assert_eq!(info.logical_rect.y.0, 0.0);
        assert_eq!(info.logical_rect.width.0, 1920.0);
        assert_eq!(info.logical_rect.height.0, 1080.0);
        assert_eq!(info.physical_size.width.0, 3840);
        assert_eq!(info.physical_size.height.0, 2160);
        assert_eq!(info.scale, 2.0);
        assert_eq!(info.transform, Transform::Normal);
    }

    #[test]
    fn without_xdg_output_wl_data_is_the_fallback() {
        let data = OutputData {
            registry_name: 3,
            scale: 1,
            transform: Transform::Normal,
            geometry_position: Some((1920, 0)),
            current_mode: Some((1920, 1080)),
            wl_name: Some("HDMI-A-1".to_owned()),
            ..OutputData::new(3)
        };
        let info = data.to_output_info().unwrap();
        assert_eq!(info.connector, "HDMI-A-1");
        assert_eq!(info.logical_rect.x.0, 1920.0);
        assert_eq!(info.logical_rect.width.0, 1920.0);
        assert_eq!(info.logical_rect.height.0, 1080.0);
    }

    #[test]
    fn rotated_fallback_divides_mode_by_scale_after_transform() {
        // 1080x1920 native panel, rotated 90, scale 2: buffer is 1920x1080,
        // logical size is 960x540.
        let data = OutputData {
            transform: Transform::Rot90,
            scale: 2,
            current_mode: Some((1080, 1920)),
            geometry_position: Some((0, 0)),
            ..OutputData::new(1)
        };
        let info = data.to_output_info().unwrap();
        assert_eq!(info.physical_size.width.0, 1080);
        assert_eq!(info.physical_size.height.0, 1920);
        assert_eq!(info.logical_rect.width.0, 960.0);
        assert_eq!(info.logical_rect.height.0, 540.0);
    }

    #[test]
    fn missing_mode_derives_physical_size_from_logical_size() {
        // Logical 1920x1080 at scale 2 with Rot90: buffer is 3840x2160, so
        // the pre-transform native size is 2160x3840.
        let data = OutputData {
            transform: Transform::Rot90,
            scale: 2,
            logical_position: Some((0, 0)),
            logical_size: Some((1920, 1080)),
            ..OutputData::new(1)
        };
        let info = data.to_output_info().unwrap();
        assert_eq!(info.physical_size.width.0, 2160);
        assert_eq!(info.physical_size.height.0, 3840);
        assert_eq!(info.buffer_size().width.0, 3840);
        assert_eq!(info.buffer_size().height.0, 2160);
    }

    #[test]
    fn non_positive_scale_is_normalized_to_one() {
        let data = OutputData {
            scale: 0,
            current_mode: Some((800, 600)),
            ..OutputData::new(1)
        };
        let info = data.to_output_info().unwrap();
        assert_eq!(info.scale, 1.0);
        assert_eq!(info.logical_rect.width.0, 800.0);
    }

    #[test]
    fn connector_falls_back_through_the_identity_chain() {
        let bare = OutputData::new(42).to_output_info().unwrap();
        assert_eq!(bare.connector, "output-42");
        assert_eq!(bare.name, "output-42");

        let with_make_model = OutputData {
            make: "DEL".to_owned(),
            model: "U2723QE".to_owned(),
            ..OutputData::new(42)
        }
        .to_output_info()
        .unwrap();
        assert_eq!(with_make_model.connector, "DEL U2723QE");
        assert_eq!(with_make_model.name, "U2723QE");

        let with_wl_name = OutputData {
            wl_name: Some("eDP-1".to_owned()),
            ..OutputData::new(42)
        }
        .to_output_info()
        .unwrap();
        assert_eq!(with_wl_name.connector, "eDP-1");
    }

    #[test]
    fn negative_mode_size_is_a_typed_geometry_error() {
        let data = OutputData {
            current_mode: Some((-1, 1080)),
            ..OutputData::new(1)
        };
        assert!(matches!(
            data.to_output_info(),
            Err(GeometryError::NegativePhysical(_))
        ));
    }
}
