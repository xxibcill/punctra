use point_workspace::PointRowLimits;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Hard ceilings for one complete Terrain Derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerrainLimits {
    point_rows: PointRowLimits,
    max_input_points: u64,
    max_faces: u64,
    max_working_bytes: u64,
    max_surface_bytes: u64,
    max_work_units: u64,
}

impl TerrainLimits {
    /// Creates explicit Derivation ceilings without hidden fallback behavior.
    #[must_use]
    pub const fn new(
        point_rows: PointRowLimits,
        max_input_points: u64,
        max_faces: u64,
        max_working_bytes: u64,
        max_surface_bytes: u64,
        max_work_units: u64,
    ) -> Self {
        Self {
            point_rows,
            max_input_points,
            max_faces,
            max_working_bytes,
            max_surface_bytes,
            max_work_units,
        }
    }

    /// Returns the complete Snapshot Point-row stream ceilings.
    #[must_use]
    pub const fn point_rows(self) -> PointRowLimits {
        self.point_rows
    }

    /// Returns the maximum exact Ground Input rows and retained vertices.
    #[must_use]
    pub const fn max_input_points(self) -> u64 {
        self.max_input_points
    }

    /// Returns the maximum canonical face count.
    #[must_use]
    pub const fn max_faces(self) -> u64 {
        self.max_faces
    }

    /// Returns the combined peak incremental Derivation byte ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }

    /// Returns the retained immutable Surface byte ceiling.
    #[must_use]
    pub const fn max_surface_bytes(self) -> u64 {
        self.max_surface_bytes
    }

    /// Returns the complete deterministic Derivation operation ceiling.
    #[must_use]
    pub const fn max_work_units(self) -> u64 {
        self.max_work_units
    }
}

impl Default for TerrainLimits {
    fn default() -> Self {
        Self::new(
            PointRowLimits::default(),
            10_000_000,
            20_000_000,
            GIB,
            2 * GIB,
            2_000_000_000,
        )
    }
}

/// Hard ceilings for one detached Check Point evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct CheckPointLimits {
    max_check_points: u64,
    max_result_bytes: u64,
    max_face_tests: u64,
    max_working_bytes: u64,
}

impl CheckPointLimits {
    /// Creates explicit Check Point input, output, work, and memory ceilings.
    #[must_use]
    pub const fn new(
        max_check_points: u64,
        max_result_bytes: u64,
        max_face_tests: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            max_check_points,
            max_result_bytes,
            max_face_tests,
            max_working_bytes,
        }
    }

    /// Returns the maximum accepted detached Check Points.
    #[must_use]
    pub const fn max_check_points(self) -> u64 {
        self.max_check_points
    }

    /// Returns the maximum retained result bytes.
    #[must_use]
    pub const fn max_result_bytes(self) -> u64 {
        self.max_result_bytes
    }

    /// Returns the maximum deterministic face containment tests.
    #[must_use]
    pub const fn max_face_tests(self) -> u64 {
        self.max_face_tests
    }

    /// Returns the peak incremental Check Point working-byte ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for CheckPointLimits {
    fn default() -> Self {
        Self::new(1_000_000, 128 * MIB, 100_000_000, 256 * MIB)
    }
}

/// Hard ceilings for one metric-metre `LandXML` publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct LandXmlLimits {
    max_vertices: u64,
    max_faces: u64,
    max_output_bytes: u64,
    max_staging_bytes: u64,
    max_write_buffer_bytes: u64,
    max_xml_token_bytes: u64,
    max_working_bytes: u64,
}

impl LandXmlLimits {
    /// Creates explicit `LandXML` element, file, buffer, and working ceilings.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        max_vertices: u64,
        max_faces: u64,
        max_output_bytes: u64,
        max_staging_bytes: u64,
        max_write_buffer_bytes: u64,
        max_xml_token_bytes: u64,
        max_working_bytes: u64,
    ) -> Self {
        Self {
            max_vertices,
            max_faces,
            max_output_bytes,
            max_staging_bytes,
            max_write_buffer_bytes,
            max_xml_token_bytes,
            max_working_bytes,
        }
    }

    /// Returns the maximum emitted `LandXML` points.
    #[must_use]
    pub const fn max_vertices(self) -> u64 {
        self.max_vertices
    }

    /// Returns the maximum emitted `LandXML` faces.
    #[must_use]
    pub const fn max_faces(self) -> u64 {
        self.max_faces
    }

    /// Returns the maximum complete XML byte length.
    #[must_use]
    pub const fn max_output_bytes(self) -> u64 {
        self.max_output_bytes
    }

    /// Returns the maximum disposable staging-file bytes.
    #[must_use]
    pub const fn max_staging_bytes(self) -> u64 {
        self.max_staging_bytes
    }

    /// Returns the maximum encoder write-buffer bytes.
    #[must_use]
    pub const fn max_write_buffer_bytes(self) -> u64 {
        self.max_write_buffer_bytes
    }

    /// Returns the maximum encoded bytes in one XML token.
    #[must_use]
    pub const fn max_xml_token_bytes(self) -> u64 {
        self.max_xml_token_bytes
    }

    /// Returns the peak incremental export working-byte ceiling.
    #[must_use]
    pub const fn max_working_bytes(self) -> u64 {
        self.max_working_bytes
    }
}

impl Default for LandXmlLimits {
    fn default() -> Self {
        Self::new(
            10_000_000,
            20_000_000,
            4 * GIB,
            4 * GIB,
            MIB,
            4 * 1024,
            8 * MIB,
        )
    }
}

#[cfg(test)]
mod tests {
    use point_workspace::PointRowLimits;

    use super::{CheckPointLimits, LandXmlLimits, TerrainLimits};

    #[test]
    fn limits_preserve_independent_zero_ceilings() {
        let terrain = TerrainLimits::new(PointRowLimits::default(), 0, 0, 0, 0, 0);
        assert_eq!(terrain.max_input_points(), 0);
        assert_eq!(terrain.max_working_bytes(), 0);

        let check_points = CheckPointLimits::new(0, 0, 0, 0);
        assert_eq!(check_points.max_face_tests(), 0);

        let xml = LandXmlLimits::new(0, 0, 0, 0, 0, 0, 0);
        assert_eq!(xml.max_output_bytes(), 0);
    }
}
