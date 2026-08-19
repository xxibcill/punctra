//! Interactive progressive renderer for the synthetic fixture or one LAS/LAZ Source.

mod appearance;
mod corpus;
mod diagnostic;
mod orbit_camera;
mod real_cloud;
mod review;
mod scene;
mod status;
mod status_overlay;
mod synthetic;

use std::{
    error::Error,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use appearance::{
    DensityTransitions, REFERENCE_POINT_SIZE_PIXELS, TransitionAction, apply_transition_action,
    projected_density_point_size, renderer_appearance_config,
};
use diagnostic::{ViewFailure, ViewPhase, classify_protocol_failure, classify_renderer_failure};
use orbit_camera::{OrbitCamera, ProjectionMode};
use point_contracts::SourceMetadata;
use point_index::{PrepareLimits, PreparedIndex, prepare_with_recipe};
use point_view::{
    AvailableNodes, PlannerConfig, PlanningBudget, ResourceUsage, ViewPlan, ViewPlanner,
};
use point_workspace::OperationId;
use real_cloud::RealCloudScene;
use render_protocol::{
    ProtocolError, RenderLimits, RenderStateModel, RenderUpdate, UpdateReport, ViewGenerationKey,
    ViewId, Viewport,
};
use render_wgpu::{
    Camera, DepthCueStatus, Frame, FrameReport, PickPoll, PickRequest, PickTicket, PointStyle,
    RecordedFrame, RendererError, WgpuRenderer,
};
use renderer_demo::display::{DisplayIndexPolicy, DisplayMode};
use review::{
    CaptureView, ClassificationEdit, MutationDisposition, ReviewCapture, ReviewOptions,
    ReviewRecovery, ReviewSession, ReviewStatus,
};
use scene::{Scene, SceneMetrics};
use status::{SelectionAction, StatusSnapshot, StreamStatus};
use status_overlay::StatusOverlay;
use synthetic::{RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const BASE_TITLE: &str = concat!("Punctra ", env!("CARGO_PKG_VERSION"), " View");
const INITIAL_WIDTH: f64 = 1_280.0;
const INITIAL_HEIGHT: f64 = 800.0;
const TRANSCRIPT_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const VIEW_GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(1), 1);
const PLANNING_BUDGET: PlanningBudget = PlanningBudget::new(
    RESIDENT_POINT_BUDGET,
    RESIDENT_BYTE_BUDGET,
    RESIDENT_BATCH_BUDGET,
);

const fn active_capture_view(interaction_generation: u64) -> CaptureView {
    CaptureView::new(VIEW_GENERATION, interaction_generation)
}

type DemoResult<T> = Result<T, Box<dyn Error>>;

fn internal_failure(phase: ViewPhase, error: impl std::fmt::Display) -> Box<dyn Error> {
    Box::new(ViewFailure::internal(phase, error))
}

fn gpu_failure(phase: ViewPhase, error: impl std::fmt::Display) -> Box<dyn Error> {
    Box::new(ViewFailure::gpu(phase, error))
}

fn protocol_failure(phase: ViewPhase, error: ProtocolError) -> Box<dyn Error> {
    Box::new(classify_protocol_failure(phase, error))
}

fn renderer_failure(phase: ViewPhase, error: RendererError) -> Box<dyn Error> {
    Box::new(classify_renderer_failure(phase, error))
}

fn preserve_failure_or_internal(phase: ViewPhase, error: Box<dyn Error>) -> Box<dyn Error> {
    if error.downcast_ref::<ViewFailure>().is_some() {
        error
    } else {
        internal_failure(phase, error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Command {
    show_help: bool,
    headless_smoke: bool,
    display_mode: DisplayMode,
    projection: ProjectionMode,
    workspace: Option<PathBuf>,
    review: ReviewOptions,
    source: Option<PathBuf>,
    index_target: Option<PathBuf>,
}

impl Command {
    #[allow(
        clippy::too_many_lines,
        reason = "one bounded OsString pass keeps option values private and order-independent"
    )]
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> DemoResult<Self> {
        let mut headless_smoke = false;
        let mut display_mode = DisplayMode::Neutral;
        let mut display_selected = false;
        let mut projection = ProjectionMode::Perspective;
        let mut projection_selected = false;
        let mut workspace = None;
        let mut operation = None;
        let mut classification = None;
        let mut classification_filter = None;
        let mut revert_operation = None;
        let mut resolve_operation = None;
        let mut source = None;
        let mut index_target = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            if argument == OsStr::new("--help") || argument == OsStr::new("-h") {
                return Ok(Self {
                    show_help: true,
                    headless_smoke: false,
                    display_mode: DisplayMode::Neutral,
                    projection: ProjectionMode::Perspective,
                    workspace: None,
                    review: ReviewOptions::default(),
                    source: None,
                    index_target: None,
                });
            } else if argument == OsStr::new("--smoke") {
                headless_smoke = true;
            } else if argument == OsStr::new("--display") {
                if display_selected {
                    return Err(invalid_argument("--display may be specified only once"));
                }
                let value = arguments
                    .next()
                    .ok_or_else(|| invalid_argument("--display requires a supported mode"))?;
                display_mode = DisplayMode::parse(&value).ok_or_else(|| {
                    invalid_argument(
                        "unsupported display mode; expected neutral, elevation, rgb, intensity, or classification",
                    )
                })?;
                display_selected = true;
            } else if argument == OsStr::new("--projection") {
                if projection_selected {
                    return Err(invalid_argument("--projection may be specified only once"));
                }
                let value = arguments.next().ok_or_else(|| {
                    invalid_argument("--projection requires perspective or orthographic")
                })?;
                projection = ProjectionMode::parse(&value).ok_or_else(|| {
                    invalid_argument("unsupported projection; expected perspective or orthographic")
                })?;
                projection_selected = true;
            } else if argument == OsStr::new("--workspace") {
                set_once(&mut workspace, arguments.next(), "--workspace", parse_path)?;
            } else if argument == OsStr::new("--operation-id") {
                set_once(
                    &mut operation,
                    arguments.next(),
                    "--operation-id",
                    parse_operation,
                )?;
            } else if argument == OsStr::new("--classification") {
                set_once(
                    &mut classification,
                    arguments.next(),
                    "--classification",
                    parse_u8,
                )?;
            } else if argument == OsStr::new("--filter-classification") {
                set_once(
                    &mut classification_filter,
                    arguments.next(),
                    "--filter-classification",
                    parse_u8,
                )?;
            } else if argument == OsStr::new("--revert-operation-id") {
                set_once(
                    &mut revert_operation,
                    arguments.next(),
                    "--revert-operation-id",
                    parse_operation,
                )?;
            } else if argument == OsStr::new("--resolve-operation-id") {
                set_once(
                    &mut resolve_operation,
                    arguments.next(),
                    "--resolve-operation-id",
                    parse_operation,
                )?;
            } else if argument == OsStr::new("--") {
                for positional in arguments {
                    push_positional(&mut source, &mut index_target, positional)?;
                }
                break;
            } else if argument.as_encoded_bytes().first() == Some(&b'-') {
                return Err(invalid_argument("unrecognized renderer-demo option"));
            } else {
                push_positional(&mut source, &mut index_target, argument)?;
            }
        }
        if display_mode.requires_source() && source.is_none() {
            return Err(invalid_argument(format_args!(
                "--display {display_mode} requires a LAS or LAZ SOURCE"
            )));
        }
        if workspace.is_some() && source.is_none() {
            return Err(invalid_argument("--workspace requires a LAS or LAZ SOURCE"));
        }
        if operation.is_some() != classification.is_some() {
            return Err(invalid_argument(
                "--operation-id and --classification must be supplied together",
            ));
        }
        if workspace.is_none()
            && (operation.is_some()
                || classification_filter.is_some()
                || revert_operation.is_some()
                || resolve_operation.is_some())
        {
            return Err(invalid_argument(
                "review, correction, and Revert options require --workspace",
            ));
        }
        let classification_edit = operation
            .zip(classification)
            .map(|(operation, value)| ClassificationEdit { operation, value });
        Ok(Self {
            show_help: false,
            headless_smoke,
            display_mode,
            projection,
            workspace,
            review: ReviewOptions {
                classification_filter,
                classification_edit,
                revert_operation,
                resolve_operation,
            },
            source,
            index_target,
        })
    }
}

fn set_once<T>(
    target: &mut Option<T>,
    value: Option<OsString>,
    option: &'static str,
    parse: impl FnOnce(Option<OsString>, &'static str) -> DemoResult<T>,
) -> DemoResult<()> {
    if target.is_some() {
        return Err(invalid_argument(format_args!(
            "{option} may be specified only once"
        )));
    }
    *target = Some(parse(value, option)?);
    Ok(())
}

fn parse_path(value: Option<OsString>, option: &'static str) -> DemoResult<PathBuf> {
    Ok(PathBuf::from(value.ok_or_else(|| {
        invalid_argument(format_args!("{option} requires a path"))
    })?))
}

fn parse_operation(value: Option<OsString>, option: &'static str) -> DemoResult<OperationId> {
    let value = value.ok_or_else(|| {
        invalid_argument(format_args!(
            "{option} requires exactly 32 hexadecimal digits"
        ))
    })?;
    let value = value.to_str().ok_or_else(|| {
        invalid_argument(format_args!("{option} requires ASCII hexadecimal digits"))
    })?;
    let bytes = decode_operation_hex(value).ok_or_else(|| {
        invalid_argument(format_args!(
            "{option} requires exactly 32 hexadecimal digits"
        ))
    })?;
    OperationId::from_bytes(bytes)
        .map_err(|_| invalid_argument(format_args!("{option} must be nonzero")))
}

fn decode_operation_hex(value: &str) -> Option<[u8; 16]> {
    let encoded = value.as_bytes();
    if encoded.len() != 32 || !encoded.is_ascii() {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (output, pair) in bytes.iter_mut().zip(encoded.chunks_exact(2)) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        *output = (high << 4) | low;
    }
    Some(bytes)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_u8(value: Option<OsString>, option: &'static str) -> DemoResult<u8> {
    value
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<u8>().ok())
        .ok_or_else(|| invalid_argument(format_args!("{option} requires an integer from 0 to 255")))
}

fn push_positional(
    source: &mut Option<PathBuf>,
    index_target: &mut Option<PathBuf>,
    value: OsString,
) -> DemoResult<()> {
    if source.is_none() {
        *source = Some(PathBuf::from(value));
    } else if index_target.is_none() {
        *index_target = Some(PathBuf::from(value));
    } else {
        return Err(invalid_argument(
            "renderer-demo accepts at most SOURCE and INDEX_TARGET",
        ));
    }
    Ok(())
}

struct LoadedDemo {
    scene: Scene,
    review: Option<ReviewSession>,
    reopen: Option<(PathBuf, PreparedIndex)>,
}

fn load_scene(command: &Command) -> DemoResult<LoadedDemo> {
    let Some(source_path) = command.source.as_deref() else {
        let scene = Scene::synthetic(VIEW_GENERATION)
            .map_err(|error| preserve_failure_or_internal(ViewPhase::Planning, error))?;
        return Ok(LoadedDemo {
            scene,
            review: None,
            reopen: None,
        });
    };
    let index_target = command.index_target.clone().unwrap_or_else(|| {
        default_index_target(
            source_path,
            command.display_mode,
            command.workspace.is_some(),
        )
    });

    let verification_started = Instant::now();
    println!("View phase: source-verification (running)");
    let source = source_las::open(source_path)
        .blocking_wait()
        .map_err(|error| ViewFailure::source(source_path, &error))?;
    let verification_elapsed = verification_started.elapsed();
    validate_display_source(command.display_mode, source.metadata())?;
    println!(
        "View phase: source-verification (complete)\nVerified Source (Full)\n  path: {}\n  identity: {}\n  Points: {}\n  verification: {:.3} s\n  \
         display: {}",
        source_path.display(),
        source.identity(),
        source.metadata().point_count(),
        verification_elapsed.as_secs_f64(),
        display_description(command.display_mode, source.metadata().world_bounds()),
    );

    let prepare_started = Instant::now();
    println!("View phase: index-prepare (running)");
    let recipe = display_index_policy(command.display_mode, command.workspace.is_some()).recipe();
    let prepared = prepare_with_recipe(source, &index_target, recipe, PrepareLimits::default())
        .blocking_wait()
        .map_err(|error| ViewFailure::index(&index_target, &error))?;
    let prepare_elapsed = prepare_started.elapsed();
    println!("View phase: index-prepare (complete)");
    print_prepare_report(&prepared, &index_target, prepare_elapsed);
    let review = command
        .workspace
        .as_deref()
        .map(|root| ReviewSession::open(root, prepared.clone(), command.review))
        .transpose()?;
    let reopen = command
        .workspace
        .as_ref()
        .map(|root| (root.clone(), prepared.clone()));
    let scene = if command.workspace.is_some() {
        RealCloudScene::new_for_review(VIEW_GENERATION, prepared, command.display_mode)?
    } else {
        RealCloudScene::new(VIEW_GENERATION, prepared, command.display_mode)?
    };
    Ok(LoadedDemo {
        scene: Scene::real(scene),
        review,
        reopen,
    })
}

fn display_description(mode: DisplayMode, bounds: Option<point_contracts::WorldBounds>) -> String {
    match (mode, bounds) {
        (DisplayMode::Neutral, _) => "neutral application color".to_owned(),
        (DisplayMode::Elevation, Some(bounds)) => format!(
            "elevation palette normalized by Source world Z bounds [{}, {}]",
            bounds.min()[2],
            bounds.max()[2]
        ),
        (DisplayMode::Elevation, None) => {
            "elevation palette (empty Source has no world Z bounds)".to_owned()
        }
        (DisplayMode::Rgb, _) => "rgb from raw Source U16 channels scaled to RGBA8".to_owned(),
        (DisplayMode::Intensity, _) => {
            "intensity from raw Source U16 scaled to grayscale RGBA8".to_owned()
        }
        (DisplayMode::Classification, _) => {
            "classification from raw Source U8 mapped by the fixed v0.10 palette".to_owned()
        }
    }
}

const fn display_index_policy(mode: DisplayMode, exact_review: bool) -> DisplayIndexPolicy {
    if exact_review {
        DisplayIndexPolicy::Inspection
    } else {
        mode.index_policy()
    }
}
fn validate_display_source(mode: DisplayMode, metadata: &SourceMetadata) -> DemoResult<()> {
    mode.validate_source(metadata)
        .map_err(|message| invalid_argument(format_args!("--display {mode}: {message}")))
}

fn print_prepare_report(index: &PreparedIndex, target: &Path, elapsed: Duration) {
    let report = index.prepare_report();
    println!(
        "Point index prepare\n  target: {}\n  disposition: {:?}\n  durable Points reused: {}\n  \
         Source Points read: {}\n  artifact bytes: {}\n  elapsed: {:.3} s",
        target.display(),
        report.disposition(),
        report.durable_points_reused(),
        report.source_points_read(),
        report.artifact_bytes(),
        elapsed.as_secs_f64(),
    );
}

fn default_index_target(source: &Path, display_mode: DisplayMode, exact_review: bool) -> PathBuf {
    let mut target = source.as_os_str().to_os_string();
    target.push(display_index_policy(display_mode, exact_review).target_suffix());
    PathBuf::from(target)
}

fn invalid_argument(message: impl std::fmt::Display) -> Box<dyn Error> {
    Box::new(ViewFailure::invalid_request(message))
}

#[allow(
    clippy::too_many_lines,
    reason = "the smoke preserves one linear evidence transcript across render and review stages"
)]
fn run_headless_smoke(loaded: LoadedDemo, projection: ProjectionMode) -> DemoResult<()> {
    let LoadedDemo {
        mut scene,
        mut review,
        reopen,
    } = loaded;
    let mut renderer = RenderStateModel::new(
        RenderLimits::new(
            RESIDENT_BYTE_BUDGET,
            RESIDENT_POINT_BUDGET,
            RESIDENT_BATCH_BUDGET,
        )
        .with_max_highlight_points(review::MAX_HIGHLIGHT_POINTS),
    );
    renderer
        .apply(&RenderUpdate::Reset {
            view_generation: VIEW_GENERATION,
        })
        .map_err(|error| protocol_failure(ViewPhase::HostStaging, error))?;
    let camera = OrbitCamera::new(scene.camera_target(), scene.camera_radius())
        .with_projection(projection)
        .as_render_camera()
        .map_err(|error| internal_failure(ViewPhase::Planning, error))?;
    let mut planner = ViewPlanner::new(
        PlannerConfig::new(2.0, 0.25)
            .map_err(|error| internal_failure(ViewPhase::Planning, error))?,
    );
    let viewport =
        Viewport::new(1_280, 800).map_err(|error| internal_failure(ViewPhase::Planning, error))?;
    let plan = {
        let nodes = scene.planning_nodes();
        planner
            .plan(
                &camera,
                viewport,
                AvailableNodes::new(VIEW_GENERATION, nodes.as_slice()),
                PLANNING_BUDGET,
            )
            .map_err(|error| internal_failure(ViewPhase::Planning, error))?
    };
    scene
        .reconcile_requests(plan.demanded_nodes(), plan.requests())
        .map_err(|error| preserve_failure_or_internal(ViewPhase::Planning, error))?;

    let batch = scene
        .next_batch()
        .map_err(|error| preserve_failure_or_internal(ViewPhase::NodeRead, error))?;
    if let Some(batch) = batch {
        let key = batch.key();
        let version = batch.version();
        let point_count = batch.point_count();
        if let Err(error) = renderer.apply(&RenderUpdate::Upsert { batch }) {
            scene.mark_rejected(key, version);
            return Err(protocol_failure(ViewPhase::HostStaging, error));
        }
        scene.mark_resident(key, version);
        let metrics = scene.metrics();
        println!(
            "Headless bridge smoke accepted one atomic Upsert\n  scene: {}\n  Points: {point_count}\n  \
             resident batches: {}\n  queued batches: {}\n  peak staging: {} Points / {} bytes",
            scene.label(),
            metrics.resident_batches,
            metrics.queued_batches,
            metrics.peak_staged_points,
            metrics.peak_staged_bytes,
        );
    } else if scene.metrics().logical_points == 0 {
        println!("Headless bridge smoke: verified empty Source; no display batch exists");
    } else {
        return Err(internal_failure(
            ViewPhase::NodeRead,
            "headless bridge smoke produced no requested root batch",
        ));
    }

    if let Some(review) = review.as_mut() {
        let highlights = review.select_full_view_blocking(camera, viewport)?;
        let exact_count = highlights.as_slice().len();
        let point_ids = highlights.into_vec();
        renderer
            .apply(&RenderUpdate::SetHighlights {
                view_generation: VIEW_GENERATION,
                point_ids: point_ids.clone(),
            })
            .map_err(|error| protocol_failure(ViewPhase::HostStaging, error))?;
        println!(
            "Headless exact review published one complete highlight update\n  exact Points: {exact_count}"
        );
        if let Some(point) = point_ids.first().copied() {
            let confirmed = review.confirm_headless(point)?;
            if confirmed.as_slice() != [point] {
                return Err(internal_failure(
                    ViewPhase::HostStaging,
                    "exact pick confirmation did not preserve the provisional Point identity",
                ));
            }
        }
        if review.has_classification_edit() {
            review
                .commit_selected(active_capture_view(0))?
                .require_committed("classification edit")?;
        }
        if review.has_revert() {
            review
                .revert_head()?
                .require_committed("immediate-head Revert")?;
        }
    }

    let terminal_head = review.as_ref().map(ReviewSession::head_revision);
    drop(review);
    if let (Some((root, index)), Some(expected)) = (reopen, terminal_head) {
        let reopened = review::reopen_head(&root, index)?;
        if reopened != expected {
            return Err(internal_failure(
                ViewPhase::HostStaging,
                "reopened Workspace head differs from the completed review state",
            ));
        }
    }
    Ok(())
}

fn print_usage() {
    println!(
        "Usage: renderer-demo [--smoke] [--display neutral|elevation|rgb|intensity|classification] \
         [--projection perspective|orthographic] [--workspace PATH] \
         [--filter-classification VALUE] \
         [--operation-id HEX32 --classification VALUE] [--revert-operation-id HEX32] \
         [--resolve-operation-id HEX32] \
         [SOURCE [INDEX_TARGET]]\n\
         With no SOURCE, runs the original synthetic scene. SOURCE must be LAS or LAZ; it is \
         Full-verified before the index is opened, resumed, or built. If INDEX_TARGET is omitted, \
         neutral/elevation use SOURCE.pidx and attributed modes use \
         SOURCE.inspection-v2.pidx. The default neutral display preserves the original behavior. \
         Elevation maps indexed positions against complete Source world Z bounds. RGB, intensity, \
         and classification use a version-2 inspection index; RGB fails when the Source lacks all \
         three U16 channels. Perspective is the default; orthographic preserves target-plane scale. \
         --workspace opens an existing Workspace only and uses a cloned attributed PreparedIndex. \
         Right click requests a provisional GPU pick followed by exact CPU confirmation; right drag \
         performs inclusive screen-through selection. C commits the selected Point Set with the \
         caller-owned operation and explicit classification; U uses only the caller-owned Revert \
         operation against the immediate head; X clears selection. No Workspace is created and no \
         stale result, mutation, retry, repin, or Revert is automatic. With --workspace, --smoke also \
         exercises exact full-viewport selection, confirmation, complete highlighting, requested \
         mutation, audit, and reopen without a GPU. --resolve-operation-id explicitly reopens and \
         reports durable state for that same caller-retained identity without retrying it."
    );
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_main() -> DemoResult<()> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    if first.as_deref() == Some(OsStr::new("corpus")) {
        let command =
            corpus::CorpusCommand::parse(arguments).map_err(ViewFailure::invalid_request)?;
        return corpus::run(&command);
    }
    let command = Command::parse(first.into_iter().chain(arguments))?;
    if command.show_help {
        print_usage();
        return Ok(());
    }
    let loaded = load_scene(&command)?;
    if command.headless_smoke {
        return run_headless_smoke(loaded, command.projection);
    }
    let LoadedDemo {
        scene,
        review,
        reopen: _,
    } = loaded;
    let scene_metrics = scene.metrics();
    println!(
        "Punctra adaptive View demo ({} {}, fixed residency)\n\
         Left drag: orbit | Middle drag: pan | Wheel: zoom | P: projection | \
         R: reset view | Right click: exact pick | Right drag: exact rectangle | \
         C: commit classification | U: immediate-head Revert | X: clear exact selection | \
         H: highlights | Space: pause loads | Escape: quit",
        compact_count(scene_metrics.logical_points),
        scene.label(),
    );

    let event_loop =
        EventLoop::new().map_err(|error| internal_failure(ViewPhase::GpuSetup, error))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = DemoApp::new(scene, review, command.projection, command.display_mode);
    event_loop
        .run_app(&mut app)
        .map_err(|error| internal_failure(ViewPhase::Rendering, error))?;
    if let Some(failure) = app.failure {
        return Err(Box::new(failure));
    }
    Ok(())
}

struct DemoApp {
    scene: Option<Scene>,
    review: Option<ReviewSession>,
    projection: ProjectionMode,
    display_mode: DisplayMode,
    graphics: Option<Graphics>,
    failure: Option<ViewFailure>,
}

impl DemoApp {
    const fn new(
        scene: Scene,
        review: Option<ReviewSession>,
        projection: ProjectionMode,
        display_mode: DisplayMode,
    ) -> Self {
        Self {
            scene: Some(scene),
            review,
            projection,
            display_mode,
            graphics: None,
            failure: None,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> DemoResult<()> {
        let attributes = Window::default_attributes()
            .with_title(initial_title())
            .with_inner_size(LogicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT))
            .with_min_inner_size(LogicalSize::new(640.0, 480.0))
            .with_visible(false);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let scene = self
            .scene
            .take()
            .ok_or_else(|| io::Error::other("demo scene was already consumed"))?;
        let review = self.review.take();
        let graphics = pollster::block_on(Graphics::new(
            instance,
            window,
            scene,
            review,
            self.projection,
            self.display_mode,
        ))?;
        graphics.window.set_visible(true);
        graphics.window.request_redraw();
        self.graphics = Some(graphics);
        Ok(())
    }

    fn fail(
        &mut self,
        event_loop: &ActiveEventLoop,
        phase: ViewPhase,
        error: &(dyn Error + 'static),
    ) {
        self.failure = Some(
            error
                .downcast_ref::<ViewFailure>()
                .map_or_else(|| ViewFailure::internal(phase, error), ViewFailure::reowned),
        );
        self.graphics = None;
        event_loop.exit();
    }

    fn handle_redraw(&mut self, event_loop: &ActiveEventLoop) {
        let result = self
            .graphics
            .as_mut()
            .expect("redraw events are filtered to the demo window")
            .redraw();
        if let Err(error) = result {
            self.fail(event_loop, ViewPhase::Rendering, error.as_ref());
            return;
        }

        let graphics = self
            .graphics
            .as_ref()
            .expect("successful redraw keeps graphics state alive");
        if graphics.should_request_redraw() {
            graphics.window.request_redraw();
        }
    }

    fn handle_keyboard_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
    ) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if code == KeyCode::Escape {
            event_loop.exit();
            return;
        }

        let result = self
            .graphics
            .as_mut()
            .expect("keyboard events are filtered to the demo window")
            .handle_key(code);
        if let Err(error) = result {
            self.fail(event_loop, ViewPhase::Planning, error.as_ref());
        }
    }

    fn is_demo_window(&self, window_id: WindowId) -> bool {
        self.graphics
            .as_ref()
            .is_some_and(|graphics| graphics.window.id() == window_id)
    }
}

impl ApplicationHandler for DemoApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() || self.failure.is_some() {
            return;
        }
        if let Err(error) = self.initialize(event_loop) {
            self.fail(event_loop, ViewPhase::GpuSetup, error.as_ref());
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.is_demo_window(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.handle_redraw(event_loop),
            WindowEvent::Resized(size) => {
                self.graphics
                    .as_mut()
                    .expect("resize events are filtered to the demo window")
                    .resize(size);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let graphics = self
                    .graphics
                    .as_mut()
                    .expect("scale events are filtered to the demo window");
                graphics.resize(graphics.window.inner_size());
            }
            WindowEvent::Occluded(occluded) => {
                let graphics = self
                    .graphics
                    .as_mut()
                    .expect("occlusion events are filtered to the demo window");
                graphics.set_occluded(occluded);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(event_loop, &event);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let result = self
                    .graphics
                    .as_mut()
                    .expect("mouse events are filtered to the demo window")
                    .handle_mouse_button(state, button);
                if let Err(error) = result {
                    self.fail(event_loop, ViewPhase::Rendering, error.as_ref());
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let result = self
                    .graphics
                    .as_mut()
                    .expect("cursor events are filtered to the demo window")
                    .handle_cursor_moved(position);
                if let Err(error) = result {
                    self.fail(event_loop, ViewPhase::Planning, error.as_ref());
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.graphics
                    .as_mut()
                    .expect("cursor events are filtered to the demo window")
                    .clear_cursor_position();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.graphics
                    .as_mut()
                    .expect("wheel events are filtered to the demo window")
                    .handle_mouse_wheel(delta);
            }
            _ => {}
        }
    }
}

struct Graphics {
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    presentation: PresentationState,
    renderer: WgpuRenderer,
    status_overlay: StatusOverlay,
    display_mode: DisplayMode,
    planner: ViewPlanner,
    scene: Scene,
    review: Option<ReviewSession>,
    review_recovery: Option<ReviewRecovery>,
    camera: OrbitCamera,
    camera_reset_radius: f64,
    style: PointStyle,
    density_transitions: DensityTransitions,
    input: PointerInput,
    loads_paused: bool,
    highlights_enabled: bool,
    interaction_generation: u64,
    latest_recorded_frame: Option<ReviewRecordedFrame>,
    pending_pick: Option<PendingPick>,
    metrics: Metrics,
}

struct ReviewRecordedFrame {
    recorded: RecordedFrame,
    interaction_generation: u64,
}

struct PendingPick {
    ticket: PickTicket,
    capture: ReviewCapture,
}

fn create_renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    limits: RenderLimits,
) -> DemoResult<WgpuRenderer> {
    let mut renderer = WgpuRenderer::new(device, renderer_appearance_config(format, limits))
        .map_err(|error| ViewFailure::gpu(ViewPhase::GpuSetup, error))?;
    if renderer.depth_cue_status() == DepthCueStatus::UnsupportedFallback {
        println!("GPU depth cue: unsupported; using the unenhanced render path");
    }
    renderer
        .apply(&RenderUpdate::Reset {
            view_generation: VIEW_GENERATION,
        })
        .map_err(|error| renderer_failure(ViewPhase::GpuSetup, error))?;
    Ok(renderer)
}

impl Graphics {
    async fn new(
        instance: wgpu::Instance,
        window: Arc<Window>,
        scene: Scene,
        review: Option<ReviewSession>,
        projection: ProjectionMode,
        display_mode: DisplayMode,
    ) -> DemoResult<Self> {
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|error| gpu_failure(ViewPhase::GpuSetup, error))?;
        println!("View phase: gpu-setup (running)");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| ViewFailure::gpu(ViewPhase::GpuSetup, error))?;
        let adapter_info = adapter.get_info();
        println!(
            "GPU: {} ({:?}, {:?})",
            adapter_info.name, adapter_info.backend, adapter_info.device_type
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("punctra renderer demo device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| ViewFailure::gpu(ViewPhase::GpuSetup, error))?;

        let size = window.inner_size();
        let surface_configured = has_area(size);
        let surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| {
                gpu_failure(
                    ViewPhase::GpuSetup,
                    "the selected adapter cannot present to the window",
                )
            })?;
        if surface_configured {
            surface.configure(&device, &surface_config);
        }

        let limits = RenderLimits::new(
            RESIDENT_BYTE_BUDGET,
            RESIDENT_POINT_BUDGET,
            RESIDENT_BATCH_BUDGET,
        )
        .with_max_highlight_points(review::MAX_HIGHLIGHT_POINTS);
        let renderer = create_renderer(&device, surface_config.format, limits)?;
        let planner = ViewPlanner::new(
            PlannerConfig::new(2.0, 0.25)
                .map_err(|error| internal_failure(ViewPhase::GpuSetup, error))?,
        );
        let camera_target = scene.camera_target();
        let camera_reset_radius = scene.camera_radius();
        let style = PointStyle::new(
            REFERENCE_POINT_SIZE_PIXELS,
            [1.0, 0.24, 0.06],
            [0.008, 0.012, 0.02, 1.0],
        )
        .map_err(|error| internal_failure(ViewPhase::GpuSetup, error))?;
        let status_overlay = StatusOverlay::new(&device, surface_config.format);

        println!("View phase: gpu-setup (complete)");
        Ok(Self {
            surface,
            window,
            instance,
            device,
            queue,
            surface_config,
            presentation: PresentationState::new(surface_configured),
            renderer,
            status_overlay,
            display_mode,
            planner,
            scene,
            review,
            review_recovery: None,
            camera: OrbitCamera::new(camera_target, camera_reset_radius)
                .with_projection(projection),
            camera_reset_radius,
            style,
            density_transitions: DensityTransitions::default(),
            input: PointerInput::default(),
            loads_paused: false,
            highlights_enabled: false,
            interaction_generation: 0,
            latest_recorded_frame: None,
            pending_pick: None,
            metrics: Metrics::new(),
        })
    }

    fn redraw(&mut self) -> DemoResult<()> {
        if !self.presentation.is_drawable() {
            return Ok(());
        }
        self.poll_review_work()?;

        let frame_started = Instant::now();
        let viewport = Viewport::new(self.surface_config.width, self.surface_config.height)
            .map_err(|error| internal_failure(ViewPhase::Planning, error))?;
        let camera = self
            .camera
            .as_render_camera()
            .map_err(|error| internal_failure(ViewPhase::Planning, error))?;
        self.plan_view(&camera, viewport)?;
        self.stream_next_batch()?;
        let Some((surface_texture, reconfigure_after_present)) = self.acquire_surface_texture()?
        else {
            return Ok(());
        };
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("punctra renderer demo frame"),
            });
        let point_size = projected_density_point_size(
            viewport,
            self.density_transitions
                .display_density_point_count(&self.scene),
        );
        let style = self
            .style
            .with_display_size_pixels(point_size)
            .map_err(|error| internal_failure(ViewPhase::Rendering, error))?;
        let frame = Frame::new(VIEW_GENERATION, camera, viewport)
            .map_err(|error| internal_failure(ViewPhase::Rendering, error))?
            .with_style(style);
        let recorded_frame = self
            .renderer
            .render(&mut encoder, &target, &frame)
            .map_err(|error| renderer_failure(ViewPhase::Rendering, error))?;
        let review_status =
            self.record_status_overlay(&mut encoder, &target, recorded_frame.report())?;
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(surface_texture);
        if !self.loads_paused {
            let transition_actions = self.density_transitions.advance_presented_frame();
            self.apply_transition_actions(transition_actions)?;
        }
        if reconfigure_after_present {
            self.configure_surface();
        }

        self.metrics
            .record_frame(recorded_frame.report(), frame_started.elapsed());
        self.latest_recorded_frame = Some(ReviewRecordedFrame {
            recorded: recorded_frame,
            interaction_generation: self.interaction_generation,
        });
        self.metrics.emit_diagnostic_transcript(
            self.scene.metrics(),
            self.loads_paused,
            self.density_transitions.is_active(),
            self.highlights_enabled,
            self.camera.projection(),
            review_status,
        );
        Ok(())
    }

    fn record_status_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        report: FrameReport,
    ) -> DemoResult<Option<ReviewStatus>> {
        let scene = self.scene.metrics();
        let active_review_status = self.review.as_ref().map(ReviewSession::status);
        let review_status =
            active_review_status.or(self.review_recovery.map(|_| ReviewStatus::Indeterminate));
        let selected_points = self.review.as_ref().map_or_else(
            || {
                self.review_recovery
                    .map_or(0, ReviewRecovery::selected_points)
            },
            ReviewSession::selected_points,
        );
        let selection_action = if self.review_recovery.is_some() {
            Some(SelectionAction::ReopenAndResolve)
        } else if self
            .review
            .as_ref()
            .is_some_and(ReviewSession::has_selection)
        {
            Some(SelectionAction::Clear)
        } else {
            None
        };
        let cursor_world = self
            .input
            .cursor
            .map(|position| [position.x, position.y])
            .map(|pixel| {
                self.camera.target_plane_world(
                    pixel,
                    [self.surface_config.width, self.surface_config.height],
                )
            })
            .transpose()
            .map_err(|error| internal_failure(ViewPhase::Rendering, error))?
            .flatten();
        let status = StatusSnapshot {
            display: self.display_mode,
            projection: self.camera.projection(),
            stream: self.metrics.stream_status(
                scene,
                self.loads_paused,
                self.density_transitions.is_active(),
            ),
            scene,
            drawn_points: report.drawn_points(),
            selected: review_status,
            selected_points,
            selection_action,
            resident_highlights: self.renderer.resident_highlight_points(),
            orientation: self
                .camera
                .north_orientation()
                .map_err(|error| internal_failure(ViewPhase::Rendering, error))?,
            scale_world_units: self
                .camera
                .world_units_for_pixels(100, self.surface_config.height),
            cursor_world,
        };
        self.status_overlay.render(
            &self.device,
            encoder,
            target,
            [self.surface_config.width, self.surface_config.height],
            self.window.scale_factor(),
            &status.lines(),
        );
        Ok(review_status)
    }

    fn poll_review_work(&mut self) -> DemoResult<()> {
        self.poll_provisional_pick()?;
        let completed = self.review.as_mut().and_then(ReviewSession::poll);
        let Some(completed) = completed else {
            return Ok(());
        };
        match completed {
            Ok(completed) => {
                let highlights = {
                    let review = self
                        .review
                        .as_mut()
                        .expect("completed review has a session");
                    match review.accept(completed, active_capture_view(self.interaction_generation))
                    {
                        Ok(highlights) => highlights,
                        Err(error) => {
                            if !review.is_stale() {
                                review.fail(error.as_ref());
                            }
                            return Ok(());
                        }
                    }
                };
                let update = RenderUpdate::SetHighlights {
                    view_generation: VIEW_GENERATION,
                    point_ids: highlights.into_vec(),
                };
                if let Err(error) = self.renderer.apply(&update) {
                    self.review
                        .as_mut()
                        .expect("accepted review has a session")
                        .fail(&error);
                    return Ok(());
                }
                self.highlights_enabled = true;
            }
            Err(error) => self
                .review
                .as_mut()
                .expect("failed review has a session")
                .fail(&error),
        }
        Ok(())
    }

    fn poll_provisional_pick(&mut self) -> DemoResult<()> {
        if self.pending_pick.is_none() {
            return Ok(());
        }
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| gpu_failure(ViewPhase::Rendering, error))?;
        let poll = self
            .pending_pick
            .as_mut()
            .expect("checked pending pick")
            .ticket
            .poll()
            .map_err(|error| gpu_failure(ViewPhase::Rendering, error))?;
        let PickPoll::Ready(hit) = poll else {
            return Ok(());
        };
        let PendingPick { capture, .. } = self
            .pending_pick
            .take()
            .expect("ready pick remains pending");
        let review = self.review.as_mut().expect("pending pick requires review");
        let Some(hit) = hit else {
            review.note_pick_miss();
            return Ok(());
        };
        let provisional = review::ProvisionalPickHint::from(hit);
        if provisional.view_generation() != capture.view_generation()
            || !review
                .is_capture_current(&capture, active_capture_view(self.interaction_generation))
        {
            review
                .stale_provisional_discarded("GPU pick View generation or pinned Revision changed");
            return Ok(());
        }
        println!(
            "Provisional GPU pick\n  Point hint: {:?}\n  View generation: {}\n  batch: {}\n  version: {}\n  authority: display hint only; exact CPU confirmation follows",
            provisional.point(),
            provisional.view_generation().generation(),
            provisional.batch().get(),
            provisional.version().get(),
        );
        if let Err(error) = review.confirm_provisional(capture, provisional) {
            review.fail(error.as_ref());
        }
        Ok(())
    }

    fn acquire_surface_texture(&mut self) -> DemoResult<Option<(wgpu::SurfaceTexture, bool)>> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Ok(Some((texture, false))),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(Some((texture, true))),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure_surface();
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Validation => Err(gpu_failure(
                ViewPhase::Rendering,
                "surface acquisition failed validation",
            )),
        }
    }

    fn stream_next_batch(&mut self) -> DemoResult<()> {
        if self.loads_paused || self.density_transitions.blocks_new_residency() {
            return Ok(());
        }
        let Some(batch) = self
            .scene
            .next_batch()
            .map_err(|error| preserve_failure_or_internal(ViewPhase::NodeRead, error))?
        else {
            return Ok(());
        };

        let batch_key = batch.key();
        let batch_version = batch.version();
        let update = RenderUpdate::Upsert { batch };
        let upload_started = Instant::now();
        let report = match self.renderer.apply(&update) {
            Ok(report) => report,
            Err(error) => {
                self.scene.mark_rejected(batch_key, batch_version);
                return Err(renderer_failure(ViewPhase::GpuUpload, error));
            }
        };
        if let Some(action) =
            self.density_transitions
                .uploaded_batch_presentation(appearance::ConditionalBatch {
                    view_generation: VIEW_GENERATION,
                    key: batch_key,
                    expected_version: batch_version,
                })
        {
            self.apply_transition_actions(vec![action])?;
        }
        self.scene.mark_resident(batch_key, batch_version);
        if self.highlights_enabled {
            self.apply_highlights()?;
        }
        self.metrics.record_upload(report, upload_started.elapsed());
        Ok(())
    }

    fn plan_view(&mut self, camera: &Camera, viewport: Viewport) -> DemoResult<()> {
        if self.loads_paused {
            self.metrics.record_plan(PlanFacts::default());
            return Ok(());
        }
        let (hierarchy, plan) = {
            let planning_nodes = self.scene.planning_nodes();
            let hierarchy = planning_nodes.as_slice().to_vec();
            let plan = self
                .planner
                .plan(
                    camera,
                    viewport,
                    AvailableNodes::new(VIEW_GENERATION, planning_nodes.as_slice()),
                    PLANNING_BUDGET,
                )
                .map_err(|error| internal_failure(ViewPhase::Planning, error))?;
            (hierarchy, plan)
        };

        let transition_actions = self.density_transitions.reconcile(&hierarchy, &plan);
        self.apply_transition_actions(transition_actions)?;
        let requests = if self.density_transitions.blocks_new_residency() {
            &[]
        } else {
            plan.requests()
        };
        let issued = self
            .scene
            .reconcile_requests(plan.demanded_nodes(), requests)
            .map_err(|error| preserve_failure_or_internal(ViewPhase::Planning, error))?;
        self.metrics
            .record_plan(PlanFacts::from_plan(&plan, issued));
        Ok(())
    }

    fn apply_transition_actions(&mut self, actions: Vec<TransitionAction>) -> DemoResult<()> {
        for action in actions {
            apply_transition_action(&mut self.renderer, &mut self.scene, action)
                .map_err(|error| renderer_failure(ViewPhase::GpuUpload, error))?;
        }
        Ok(())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.presentation.configured = has_area(size);
        self.invalidate_interaction_state();
        if !self.presentation.configured {
            return;
        }

        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.configure_surface();
        self.window.request_redraw();
    }

    fn configure_surface(&self) {
        if self.presentation.configured {
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn recreate_surface(&mut self) -> DemoResult<()> {
        let surface = self
            .instance
            .create_surface(Arc::clone(&self.window))
            .map_err(|error| gpu_failure(ViewPhase::Rendering, error))?;
        if self.presentation.configured {
            surface.configure(&self.device, &self.surface_config);
        }
        self.surface = surface;
        Ok(())
    }

    fn set_occluded(&mut self, occluded: bool) {
        self.presentation.occluded = occluded;
        if !occluded {
            self.window.request_redraw();
        }
    }

    fn should_request_redraw(&self) -> bool {
        self.presentation.is_drawable()
    }

    fn handle_key(&mut self, code: KeyCode) -> DemoResult<()> {
        match code {
            KeyCode::KeyR => {
                self.camera.reset(self.camera_reset_radius);
                self.invalidate_interaction_state();
            }
            KeyCode::KeyP => {
                self.camera.toggle_projection();
                self.invalidate_interaction_state();
            }
            KeyCode::KeyH => self.toggle_highlights()?,
            KeyCode::KeyC => self.commit_selected(),
            KeyCode::KeyU => self.revert_head(),
            KeyCode::KeyX => self.clear_exact_selection()?,
            KeyCode::Space => self.loads_paused = !self.loads_paused,
            _ => return Ok(()),
        }
        self.window.request_redraw();
        Ok(())
    }

    fn toggle_highlights(&mut self) -> DemoResult<()> {
        let previous = self.highlights_enabled;
        self.highlights_enabled = !self.highlights_enabled;
        if let Err(error) = self.apply_highlights() {
            self.highlights_enabled = previous;
            return Err(error);
        }
        Ok(())
    }

    fn apply_highlights(&mut self) -> DemoResult<()> {
        let point_ids = if self.highlights_enabled {
            match self.review.as_ref() {
                Some(review) => review
                    .selected_highlights()?
                    .map_or_else(Vec::new, review::ExactHighlights::into_vec),
                None => self.scene.highlight_ids(),
            }
        } else {
            Vec::new()
        };
        let update = RenderUpdate::SetHighlights {
            view_generation: VIEW_GENERATION,
            point_ids,
        };
        self.renderer
            .apply(&update)
            .map_err(|error| renderer_failure(ViewPhase::GpuUpload, error))?;
        Ok(())
    }

    fn commit_selected(&mut self) {
        let Some(review) = self.review.as_mut() else {
            return;
        };
        let result = review.commit_selected(active_capture_view(self.interaction_generation));
        match result {
            Ok(disposition) => self.finish_interactive_mutation(disposition),
            Err(error) => {
                let review = self
                    .review
                    .as_mut()
                    .expect("failed mutation retains its review session");
                if review.is_stale() {
                    eprintln!("exact review commit blocked: {error}");
                } else {
                    review.fail(error.as_ref());
                }
            }
        }
        self.window.request_redraw();
    }

    fn revert_head(&mut self) {
        let Some(review) = self.review.as_mut() else {
            return;
        };
        let result = review.revert_head();
        match result {
            Ok(disposition) => self.finish_interactive_mutation(disposition),
            Err(error) => self
                .review
                .as_mut()
                .expect("failed Revert retains its review session")
                .fail(error.as_ref()),
        }
        self.window.request_redraw();
    }

    fn finish_interactive_mutation(&mut self, disposition: MutationDisposition) {
        if let Some(recovery) = ReviewSession::close_if_indeterminate(&mut self.review, disposition)
        {
            self.review_recovery = Some(recovery);
            self.pending_pick = None;
            self.latest_recorded_frame = None;
        }
    }

    fn clear_exact_selection(&mut self) -> DemoResult<()> {
        if let Some(review) = self.review.as_mut() {
            review.clear_selection();
        }
        if let Some(recovery) = self.review_recovery.as_mut() {
            recovery.clear_selection();
        }
        self.highlights_enabled = false;
        self.renderer
            .apply(&RenderUpdate::SetHighlights {
                view_generation: VIEW_GENERATION,
                point_ids: Vec::new(),
            })
            .map_err(|error| renderer_failure(ViewPhase::GpuUpload, error))?;
        Ok(())
    }

    fn handle_mouse_button(&mut self, state: ElementState, button: MouseButton) -> DemoResult<()> {
        if button == MouseButton::Right && self.review.is_some() {
            return self.handle_review_button(state);
        }
        self.input.update_drag(state, button);
        Ok(())
    }

    fn handle_review_button(&mut self, state: ElementState) -> DemoResult<()> {
        match state {
            ElementState::Pressed => {
                let Some(cursor) = self.input.cursor else {
                    return Ok(());
                };
                self.input.review_start = Some(cursor);
            }
            ElementState::Released => {
                let Some(first) = self.input.review_start.take() else {
                    return Ok(());
                };
                let second = self.input.cursor.unwrap_or(first);
                let distance = (second.x - first.x).hypot(second.y - first.y);
                if distance <= 3.0 {
                    self.start_provisional_pick(second)?;
                } else {
                    self.start_screen_review(first, second)?;
                }
            }
        }
        Ok(())
    }

    fn start_provisional_pick(&mut self, position: PhysicalPosition<f64>) -> DemoResult<()> {
        if self.pending_pick.is_some() || self.review.as_ref().is_some_and(ReviewSession::is_busy) {
            return Ok(());
        }
        if position.x < 0.0
            || position.y < 0.0
            || position.x >= f64::from(self.surface_config.width)
            || position.y >= f64::from(self.surface_config.height)
        {
            return Ok(());
        }
        let Some(frame) = self.latest_recorded_frame.as_ref() else {
            return Ok(());
        };
        if frame.interaction_generation != self.interaction_generation {
            return Ok(());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pixel = [position.x.floor() as u32, position.y.floor() as u32];
        let capture = self
            .review
            .as_ref()
            .expect("review pick requires session")
            .capture(active_capture_view(self.interaction_generation));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("renderer-demo provisional pick"),
            });
        let ticket = self
            .renderer
            .pick(&mut encoder, &frame.recorded, PickRequest::new(pixel))
            .map_err(|error| renderer_failure(ViewPhase::Rendering, error))?;
        self.queue.submit([encoder.finish()]);
        self.pending_pick = Some(PendingPick { ticket, capture });
        self.review
            .as_mut()
            .expect("review pick requires session")
            .note_provisional_pick();
        self.window.request_redraw();
        Ok(())
    }

    fn start_screen_review(
        &mut self,
        first: PhysicalPosition<f64>,
        second: PhysicalPosition<f64>,
    ) -> DemoResult<()> {
        let Some(review) = self.review.as_mut() else {
            return Ok(());
        };
        if review.is_busy() || self.pending_pick.is_some() {
            return Ok(());
        }
        let viewport = Viewport::new(self.surface_config.width, self.surface_config.height)?;
        let camera = self.camera.as_render_camera()?;
        let capture = review.capture(active_capture_view(self.interaction_generation));
        if let Err(error) = review.select_screen(
            capture,
            camera,
            viewport,
            [first.x, first.y],
            [second.x, second.y],
        ) {
            review.fail(error.as_ref());
        }
        self.window.request_redraw();
        Ok(())
    }

    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> DemoResult<()> {
        self.input.cursor = Some(position);
        if let Some(action) = self.input.drag {
            if let Some(previous) = self.input.last_cursor {
                let horizontal = position.x - previous.x;
                let vertical = position.y - previous.y;
                match action {
                    DragAction::Orbit => self.camera.orbit(horizontal, vertical),
                    DragAction::Pan => {
                        self.camera
                            .pan(horizontal, vertical, self.surface_config.height)
                            .map_err(|error| internal_failure(ViewPhase::Planning, error))?;
                    }
                }
                self.invalidate_interaction_state();
                self.window.request_redraw();
            }
            self.input.last_cursor = Some(position);
        }
        Ok(())
    }

    fn clear_cursor_position(&mut self) {
        self.input.last_cursor = None;
        self.input.cursor = None;
    }

    fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, vertical) => f64::from(vertical),
            MouseScrollDelta::PixelDelta(position) => position.y / 80.0,
        };
        self.camera.zoom(lines);
        self.invalidate_interaction_state();
        self.window.request_redraw();
    }

    fn invalidate_interaction_state(&mut self) {
        self.interaction_generation = self.interaction_generation.saturating_add(1);
        self.latest_recorded_frame = None;
        if let Some(review) = self.review.as_mut() {
            review.invalidate_selection_view(active_capture_view(self.interaction_generation));
        }
    }
}

#[derive(Default)]
struct PointerInput {
    drag: Option<DragAction>,
    last_cursor: Option<PhysicalPosition<f64>>,
    cursor: Option<PhysicalPosition<f64>>,
    review_start: Option<PhysicalPosition<f64>>,
}

impl PointerInput {
    fn update_drag(&mut self, state: ElementState, button: MouseButton) {
        let Some(action) = DragAction::from_button(button) else {
            return;
        };
        match state {
            ElementState::Pressed => self.drag = Some(action),
            ElementState::Released if self.drag == Some(action) => self.drag = None,
            ElementState::Released => return,
        }
        self.last_cursor = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DragAction {
    Orbit,
    Pan,
}

impl DragAction {
    const fn from_button(button: MouseButton) -> Option<Self> {
        match button {
            MouseButton::Left => Some(Self::Orbit),
            MouseButton::Middle => Some(Self::Pan),
            _ => None,
        }
    }
}

struct PresentationState {
    configured: bool,
    occluded: bool,
}

impl PresentationState {
    const fn new(configured: bool) -> Self {
        Self {
            configured,
            occluded: false,
        }
    }

    const fn is_drawable(&self) -> bool {
        self.configured && !self.occluded
    }
}

struct Metrics {
    interval_started: Instant,
    interval_frames: u32,
    total_uploaded_bytes: u64,
    latest_upload_time: Duration,
    latest_frame_time: Duration,
    latest_report: Option<FrameReport>,
    latest_plan: PlanFacts,
}

impl Metrics {
    fn new() -> Self {
        Self {
            interval_started: Instant::now(),
            interval_frames: 0,
            total_uploaded_bytes: 0,
            latest_upload_time: Duration::ZERO,
            latest_frame_time: Duration::ZERO,
            latest_report: None,
            latest_plan: PlanFacts::default(),
        }
    }

    fn record_upload(&mut self, report: UpdateReport, elapsed: Duration) {
        self.total_uploaded_bytes = self
            .total_uploaded_bytes
            .saturating_add(report.uploaded_bytes());
        self.latest_upload_time = elapsed;
    }

    fn record_frame(&mut self, report: FrameReport, elapsed: Duration) {
        self.interval_frames = self.interval_frames.saturating_add(1);
        self.latest_frame_time = elapsed;
        self.latest_report = Some(report);
    }

    fn record_plan(&mut self, facts: PlanFacts) {
        self.latest_plan = facts;
    }

    const fn stream_status(
        &self,
        scene: SceneMetrics,
        loads_paused: bool,
        transition_active: bool,
    ) -> StreamStatus {
        StreamStatus::from_facts(
            scene,
            loads_paused,
            self.latest_plan.has_stream_work(),
            transition_active,
        )
    }

    fn emit_diagnostic_transcript(
        &mut self,
        scene: SceneMetrics,
        loads_paused: bool,
        transition_active: bool,
        highlights_enabled: bool,
        projection: ProjectionMode,
        review_status: Option<ReviewStatus>,
    ) {
        let interval = self.interval_started.elapsed();
        if interval < TRANSCRIPT_REFRESH_INTERVAL {
            return;
        }
        let Some(report) = self.latest_report else {
            return;
        };

        let frames_per_second = f64::from(self.interval_frames) / interval.as_secs_f64();
        let stream_state = self
            .stream_status(scene, loads_paused, transition_active)
            .label();
        let highlight_state = if highlights_enabled { "on" } else { "off" };
        let review_state =
            review_status.map_or_else(|| "review:disabled".to_owned(), ReviewStatus::title);
        let coverage_state = coverage_state(scene);
        let transcript = format!(
            "View diagnostics | {} logical | {} / {} pts | {} MiB resident | {} MiB uploaded | \
             {} draws | \
             {:.0} fps | frame {:.2} ms | encode {:.2} ms | upload {:.2} ms | \
             LOD {} demanded / {} candidates / {} issued / {} retained / {} retired-now | \
             {} planned | {} queued (peak {}) | {} staged pts (peak {}) / {} MiB peak | \
             {} requested / {} resident nodes ({} pts) | {} retired / {} rejected / {} cancelled | \
             {coverage_state} | projection:{projection} | {review_state} | {stream_state} | H:{highlight_state}",
            compact_count(scene.logical_points),
            compact_count(report.drawn_points()),
            compact_count(RESIDENT_POINT_BUDGET),
            mebibytes(report.resident_bytes()),
            mebibytes(self.total_uploaded_bytes),
            report.draw_calls(),
            frames_per_second,
            duration_milliseconds(self.latest_frame_time),
            duration_milliseconds(report.encoding_time()),
            duration_milliseconds(self.latest_upload_time),
            self.latest_plan.demanded,
            self.latest_plan.load_candidates,
            self.latest_plan.issued,
            self.latest_plan.retained,
            self.latest_plan.retirements,
            self.latest_plan.usage.batch_count(),
            scene.queued_batches,
            scene.peak_queued_batches,
            scene.staged_points,
            scene.peak_staged_points,
            mebibytes(scene.peak_staged_bytes.max(scene.staged_bytes)),
            scene.requested_nodes,
            scene.resident_batches,
            compact_count(scene.resident_points),
            scene.retired_batches,
            scene.rejected_batches,
            scene.cancelled_requests,
        );
        println!("{transcript}");
        self.interval_started = Instant::now();
        self.interval_frames = 0;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PlanFacts {
    demanded: u64,
    load_candidates: u64,
    issued: u64,
    retained: u64,
    retirements: u64,
    usage: ResourceUsage,
}

impl PlanFacts {
    fn from_plan(plan: &ViewPlan, issued: u64) -> Self {
        let count = |length: usize| u64::try_from(length).unwrap_or(u64::MAX);
        Self {
            demanded: count(plan.demanded_nodes().len()),
            load_candidates: count(plan.requests().len()),
            issued,
            retained: count(plan.retained_nodes().len()),
            retirements: count(plan.retirements().len()),
            usage: plan.resource_usage(),
        }
    }

    const fn has_stream_work(self) -> bool {
        self.demanded > 0 || self.load_candidates > 0 || self.issued > 0 || self.retirements > 0
    }
}

fn coverage_state(scene: SceneMetrics) -> String {
    if scene.authored_resident_batches > 0 {
        return format!(
            "display coverage: Sampled {} batches / {} pts; Complete {} batches / {} pts; authored fixture presentation {} batches / {} pts (not Query completion)",
            scene.sampled_resident_batches,
            compact_count(scene.sampled_resident_points),
            scene.complete_resident_batches,
            compact_count(scene.complete_resident_points),
            scene.authored_resident_batches,
            compact_count(scene.authored_resident_points)
        );
    }
    format!(
        "display coverage: Sampled {} batches / {} pts; Complete {} batches / {} pts (not Query completion)",
        scene.sampled_resident_batches,
        compact_count(scene.sampled_resident_points),
        scene.complete_resident_batches,
        compact_count(scene.complete_resident_points),
    )
}

const fn initial_title() -> &'static str {
    BASE_TITLE
}

fn has_area(size: PhysicalSize<u32>) -> bool {
    size.width > 0 && size.height > 0
}

fn duration_milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn mebibytes(bytes: u64) -> String {
    const MEBIBYTE: u64 = 1_024 * 1_024;
    let tenths = bytes.saturating_mul(10) / MEBIBYTE;
    format!("{}.{:01}", tenths / 10, tenths % 10)
}

fn compact_count(count: u64) -> String {
    const MILLION: u64 = 1_000_000;
    const THOUSAND: u64 = 1_000;
    if count >= MILLION {
        let tenths = count.saturating_mul(10) / MILLION;
        format!("{}.{:01}M", tenths / 10, tenths % 10)
    } else if count >= THOUSAND {
        let tenths = count.saturating_mul(10) / THOUSAND;
        format!("{}.{:01}K", tenths / 10, tenths % 10)
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::diagnostic::{RecoveryAction, ViewFailureCode};
    use crate::synthetic::{SCENE_RADIUS, SCENE_TARGET, SyntheticScene};

    use super::*;

    #[test]
    fn command_preserves_synthetic_default_and_accepts_real_cloud_paths() {
        let default = Command::parse(Vec::<OsString>::new()).unwrap();
        assert!(!default.headless_smoke);
        assert_eq!(default.display_mode, DisplayMode::Neutral);
        assert_eq!(default.projection, ProjectionMode::Perspective);
        assert_eq!(default.source, None);

        let real = Command::parse([
            OsString::from("--smoke"),
            OsString::from("--display"),
            OsString::from("elevation"),
            OsString::from("--projection"),
            OsString::from("orthographic"),
            OsString::from("survey.laz"),
            OsString::from("cache/survey.pidx"),
        ])
        .unwrap();
        assert!(real.headless_smoke);
        assert_eq!(real.display_mode, DisplayMode::Elevation);
        assert_eq!(real.projection, ProjectionMode::Orthographic);
        assert_eq!(real.source, Some(PathBuf::from("survey.laz")));
        assert_eq!(real.index_target, Some(PathBuf::from("cache/survey.pidx")));
    }

    #[test]
    fn command_rejects_invalid_or_inapplicable_display_selection() {
        let unsupported = Command::parse([
            OsString::from("--display"),
            OsString::from("height"),
            OsString::from("survey.las"),
        ])
        .unwrap_err();
        assert!(unsupported.to_string().contains("classification"));

        let synthetic =
            Command::parse([OsString::from("--display"), OsString::from("elevation")]).unwrap_err();
        assert!(
            synthetic
                .to_string()
                .contains("requires a LAS or LAZ SOURCE")
        );

        for mode in ["rgb", "intensity", "classification"] {
            assert!(
                Command::parse([OsString::from("--display"), OsString::from(mode)])
                    .unwrap_err()
                    .to_string()
                    .contains("requires a LAS or LAZ SOURCE")
            );
        }
    }

    #[test]
    fn command_rejects_invalid_or_repeated_projection_selection() {
        let missing = Command::parse([OsString::from("--projection")]).unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("requires perspective or orthographic")
        );

        let unsupported =
            Command::parse([OsString::from("--projection"), OsString::from("isometric")])
                .unwrap_err();
        assert!(
            unsupported
                .to_string()
                .contains("expected perspective or orthographic")
        );

        let repeated = Command::parse([
            OsString::from("--projection"),
            OsString::from("perspective"),
            OsString::from("--projection"),
            OsString::from("orthographic"),
        ])
        .unwrap_err();
        assert!(repeated.to_string().contains("specified only once"));
    }

    #[test]
    fn command_accepts_explicit_review_identities_and_values() {
        let command = Command::parse([
            OsString::from("--workspace"),
            OsString::from("survey.workspace"),
            OsString::from("--filter-classification"),
            OsString::from("2"),
            OsString::from("--operation-id"),
            OsString::from("01010101010101010101010101010101"),
            OsString::from("--classification"),
            OsString::from("1"),
            OsString::from("--revert-operation-id"),
            OsString::from("02020202020202020202020202020202"),
            OsString::from("--resolve-operation-id"),
            OsString::from("03030303030303030303030303030303"),
            OsString::from("survey.las"),
        ])
        .unwrap();

        assert_eq!(command.workspace, Some(PathBuf::from("survey.workspace")));
        assert_eq!(command.review.classification_filter, Some(2));
        assert_eq!(command.review.classification_edit.unwrap().value, 1);
        assert_eq!(
            command.review.resolve_operation.unwrap().into_bytes(),
            [3; 16]
        );
    }

    #[test]
    fn command_rejects_repeated_review_options_before_reparsing() {
        for arguments in [
            vec!["--workspace", "first", "--workspace"],
            vec![
                "--filter-classification",
                "2",
                "--filter-classification",
                "not-an-integer",
            ],
            vec![
                "--resolve-operation-id",
                "03030303030303030303030303030303",
                "--resolve-operation-id",
                "not-an-operation",
            ],
        ] {
            let error = Command::parse(arguments.into_iter().map(OsString::from)).unwrap_err();
            assert!(error.to_string().contains("specified only once"));
        }
    }

    #[test]
    fn command_rejects_implicit_review_policy_and_invalid_identities() {
        for arguments in [
            vec!["--workspace", "workspace"],
            vec![
                "--operation-id",
                "01010101010101010101010101010101",
                "survey.las",
            ],
            vec!["--classification", "2", "survey.las"],
            vec![
                "--workspace",
                "workspace",
                "--operation-id",
                "00000000000000000000000000000000",
                "--classification",
                "2",
                "survey.las",
            ],
            vec![
                "--workspace",
                "workspace",
                "--classification",
                "256",
                "survey.las",
            ],
        ] {
            assert!(Command::parse(arguments.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn operation_identity_parser_rejects_non_ascii_without_panicking() {
        let thirty_two_utf8_bytes = format!("€{}", "0".repeat(29));
        assert_eq!(thirty_two_utf8_bytes.len(), 32);
        assert_eq!(decode_operation_hex(&thirty_two_utf8_bytes), None);

        let error = Command::parse([
            OsString::from("--workspace"),
            OsString::from("workspace"),
            OsString::from("--resolve-operation-id"),
            OsString::from(thirty_two_utf8_bytes),
            OsString::from("survey.las"),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("32 hexadecimal digits"));
    }

    #[test]
    fn command_rejects_extra_positional_paths() {
        let error = Command::parse([
            OsString::from("one.las"),
            OsString::from("two.pidx"),
            OsString::from("unexpected"),
        ])
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("at most SOURCE and INDEX_TARGET")
        );
    }

    #[test]
    fn command_rejects_large_untrusted_options_without_echoing_them() {
        let large = format!("--{}", "x".repeat(1024 * 1024));
        let failure = Command::parse([OsString::from(&large)]).unwrap_err();
        let message = failure.to_string();

        assert_eq!(message.matches('x').count(), 0);
        assert!(message.contains("unrecognized renderer-demo option"));
    }

    #[cfg(unix)]
    #[test]
    fn command_checks_invalid_unicode_option_prefix_without_lossy_allocation() {
        use std::os::unix::ffi::OsStringExt as _;

        let mut bytes = Vec::with_capacity(1024 * 1024);
        bytes.push(b'-');
        bytes.resize(1024 * 1024, 0xff);
        let failure = Command::parse([OsString::from_vec(bytes)]).unwrap_err();

        let message = failure.to_string();
        assert!(message.starts_with(ViewFailureCode::InvalidRequest.as_str()));
        assert!(message.contains("unrecognized renderer-demo option"));
    }

    #[test]
    fn default_index_targets_preserve_the_source_path_and_separate_recipes() {
        assert_eq!(
            default_index_target(Path::new("survey.laz"), DisplayMode::Neutral, false),
            PathBuf::from("survey.laz.pidx")
        );
        assert_eq!(
            default_index_target(Path::new("survey.laz"), DisplayMode::Classification, false,),
            PathBuf::from("survey.laz.inspection-v2.pidx")
        );
        assert_eq!(
            default_index_target(Path::new("survey.laz"), DisplayMode::Neutral, true),
            PathBuf::from("survey.laz.inspection-v2.pidx")
        );
    }

    #[test]
    fn metrics_format_counts_without_losing_integer_precision() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(12_345), "12.3K");
        assert_eq!(compact_count(1_234_567), "1.2M");
        assert_eq!(mebibytes(3 * 1_024 * 1_024 + 512 * 1_024), "3.5");
    }

    #[test]
    fn paused_plan_facts_report_no_planner_work() {
        let scene = SyntheticScene::new(VIEW_GENERATION).unwrap();
        let nodes = scene.planning_nodes();
        let camera = OrbitCamera::new(SCENE_TARGET, SCENE_RADIUS)
            .as_render_camera()
            .unwrap();
        let plan = ViewPlanner::default()
            .plan(
                &camera,
                Viewport::new(1_280, 800).unwrap(),
                AvailableNodes::new(VIEW_GENERATION, &nodes),
                PLANNING_BUDGET,
            )
            .unwrap();
        assert!(!plan.requests().is_empty());

        let running = PlanFacts::from_plan(&plan, 1);
        let paused = PlanFacts::default();
        let demanded_only = PlanFacts {
            demanded: 1,
            ..PlanFacts::default()
        };

        assert_eq!(running.issued, running.load_candidates);
        assert_eq!(paused.demanded, 0);
        assert_eq!(paused.load_candidates, 0);
        assert_eq!(paused.issued, 0);
        assert_eq!(paused.retirements, 0);
        assert!(!paused.has_stream_work());
        assert!(demanded_only.has_stream_work());
    }

    #[test]
    fn releasing_an_inactive_drag_button_preserves_the_active_navigation() {
        let mut input = PointerInput::default();
        input.update_drag(ElementState::Pressed, MouseButton::Left);
        input.last_cursor = Some(PhysicalPosition::new(20.0, 30.0));

        input.update_drag(ElementState::Released, MouseButton::Middle);
        assert_eq!(input.drag, Some(DragAction::Orbit));
        assert_eq!(input.last_cursor, Some(PhysicalPosition::new(20.0, 30.0)));

        input.update_drag(ElementState::Released, MouseButton::Left);
        assert_eq!(input.drag, None);
        assert_eq!(input.last_cursor, None);
    }

    #[test]
    fn coverage_copy_always_names_both_kinds_without_claiming_query_completion() {
        let sampled = SceneMetrics {
            sampled_resident_batches: 2,
            sampled_resident_points: 4_096,
            complete_resident_batches: 1,
            complete_resident_points: 16,
            ..SceneMetrics::default()
        };

        let label = coverage_state(sampled);
        assert!(label.contains("Sampled 2 batches"));
        assert!(label.contains("Complete 1 batches"));
        assert!(label.contains("not Query completion"));

        let empty = coverage_state(SceneMetrics::default());
        assert!(empty.contains("Sampled 0 batches"));
        assert!(empty.contains("Complete 0 batches"));
        assert!(empty.contains("not Query completion"));

        let authored = coverage_state(SceneMetrics {
            authored_resident_batches: 1,
            authored_resident_points: 32,
            ..SceneMetrics::default()
        });
        assert!(authored.contains("Sampled 0 batches"));
        assert!(authored.contains("Complete 0 batches"));
        assert!(authored.contains("authored fixture presentation 1 batches / 32 pts"));
        assert!(authored.contains("not Query completion"));
    }

    #[test]
    fn window_title_stays_compact_and_leaves_diagnostics_to_the_transcript() {
        let title = initial_title();

        assert_eq!(title, BASE_TITLE);
        assert!(title.len() <= 64);
        assert!(!title.contains("pre-v0.13"));
        assert!(!title.contains("COVERAGE"));
        assert!(!title.contains("LOD"));
    }

    #[test]
    fn operation_failures_keep_the_owning_phase_and_resource_category() {
        let typed = Box::new(ViewFailure::resource(
            ViewPhase::NodeRead,
            "node buffer exceeded",
        ));
        let preserved = preserve_failure_or_internal(ViewPhase::Planning, typed);
        let preserved = preserved.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(preserved.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(preserved.phase(), ViewPhase::NodeRead);

        let protocol = protocol_failure(
            ViewPhase::GpuUpload,
            ProtocolError::ResidentLimitExceeded {
                resource: render_protocol::ResidentResource::Points,
                limit: 10,
                attempted: 11,
            },
        );
        let protocol = protocol.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(protocol.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(protocol.phase(), ViewPhase::GpuUpload);
        assert_eq!(protocol.action(), RecoveryAction::RaiseNamedLimit);

        let highlight = protocol_failure(
            ViewPhase::GpuUpload,
            ProtocolError::HighlightLimitExceeded {
                limit: 10,
                attempted: 11,
            },
        );
        let highlight = highlight.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(highlight.code(), ViewFailureCode::ResourceLimit);

        let untyped = preserve_failure_or_internal(
            ViewPhase::Planning,
            Box::new(io::Error::other("planner invariant")),
        );
        let untyped = untyped.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(untyped.code(), ViewFailureCode::Internal);
        assert_eq!(untyped.phase(), ViewPhase::Planning);
    }
}
