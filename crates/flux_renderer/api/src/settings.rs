/// The application's identity, reported to the graphics driver at startup.
#[derive(Debug, Clone, flux_ecs::Component)]
pub struct AppInfo {
    pub name: String,
    pub version: AppVersion,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "Flux Application".to_owned(),
            version: AppVersion::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AppVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Behavior configuration supplied by the application. Presentation
/// preferences and capability overrides are added here as the renderer gains
/// them.
#[derive(Debug, Clone, Default, flux_ecs::Component)]
pub struct RendererSettings {}
