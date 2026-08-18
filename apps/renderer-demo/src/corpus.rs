//! Bounded local field-corpus manifest and canonical viewing-report contracts.

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    marker::PhantomData,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, IgnoredAny, SeqAccess, Visitor},
};

use crate::{
    PLANNING_BUDGET, VIEW_GENERATION,
    appearance::{
        DensityTransitions, REFERENCE_POINT_SIZE_PIXELS, TransitionAction,
        projected_spacing_point_size,
    },
    diagnostic::{ViewFailure, ViewFailureCode, ViewPhase, classify_renderer_failure},
    orbit_camera::{OrbitCamera, ProjectionMode},
    real_cloud::{
        HIERARCHY_BYTE_BUDGET, QUEUED_HOST_BYTE_BUDGET, QUEUED_NODE_BUDGET, RealCloudScene,
        STAGING_BYTE_BUDGET, STAGING_POINT_BUDGET,
    },
    scene::{Scene, SceneMetrics},
};
use point_index::{
    NodeReadBudget, PrepareDisposition, PrepareLimits, PreparedIndex, prepare_fresh_with_recipe,
    prepare_with_recipe,
};
use point_view::{AvailableNodes, PlannerConfig, ViewPlanner};
use render_protocol::{RenderLimits, RenderUpdate, ResidentStats, ViewGenerationKey, Viewport};
use render_wgpu::{
    DepthCueStatus, EyeDomeLighting, Frame, PointStyle, RendererConfig, WgpuRenderer,
};
use renderer_demo::display::DisplayMode;

const MANIFEST_SCHEMA: &str = "punctra.renderer-demo.field-corpus.v1";
const REPORT_SCHEMA: &str = "punctra.renderer-demo.viewing-report.v1";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_REPORT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_TRACE_STEPS: usize = 128;
const DEFAULT_FRAMES_PER_POSE: u32 = 8;
const MAX_FRAMES_PER_POSE: u32 = 256;
const DEFAULT_SETTLEMENT_FRAME_CEILING: u32 = 1_024;
const MAX_SETTLEMENT_FRAME_CEILING: u32 = 1_024;
const SETTLED_OBSERVATION_FRAMES: u32 = 300;
const MAX_KNOWN_FEATURE_OUTCOMES: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_MANIFEST_STRING_TOKEN_BYTES: usize = MAX_PATH_BYTES * 6;
const MAX_STAGE_ATTEMPTS: usize = 128;
// `source-las` fixes this bound for Full-verification decode work in v0.10.
// The private corpus schema records that effective adapter limit without
// exposing a new foundation interface for one host.
const SOURCE_VERIFICATION_WORKING_BYTES: u64 = 64 * 1_024 * 1_024;

static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CorpusCommand {
    manifest: PathBuf,
    report: PathBuf,
}

impl CorpusCommand {
    pub(crate) fn parse(
        arguments: impl IntoIterator<Item = std::ffi::OsString>,
    ) -> io::Result<Self> {
        let mut manifest = None;
        let mut report = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            if argument == std::ffi::OsStr::new("--manifest") {
                if manifest.is_some() {
                    return invalid("--manifest may be specified only once");
                }
                manifest = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--manifest requires a path")
                })?));
            } else if argument == std::ffi::OsStr::new("--report") {
                if report.is_some() {
                    return invalid("--report may be specified only once");
                }
                report = Some(PathBuf::from(arguments.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--report requires a path")
                })?));
            } else {
                return invalid("unrecognized corpus argument; expected --manifest or --report");
            }
        }
        let manifest = manifest.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "corpus requires --manifest")
        })?;
        let report = report.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "corpus requires --report")
        })?;
        validate_path("corpus manifest path", &manifest)?;
        validate_path("viewing report path", &report)?;
        if manifest == report {
            return invalid("corpus manifest and report paths must differ");
        }
        Ok(Self { manifest, report })
    }

    pub(crate) fn manifest(&self) -> &Path {
        &self.manifest
    }

    pub(crate) fn report(&self) -> &Path {
        &self.report
    }
}

pub(crate) fn run(command: &CorpusCommand) -> Result<(), Box<dyn Error>> {
    run_with_gpu(command, || pollster::block_on(CorpusGpu::new()))
}

fn run_with_gpu(
    command: &CorpusCommand,
    acquire_gpu: impl FnOnce() -> Result<CorpusGpu, ViewFailure>,
) -> Result<(), Box<dyn Error>> {
    run_with_gpu_setup(command, acquire_gpu, |gpu| {
        let limits = RenderLimits::new(
            crate::synthetic::RESIDENT_BYTE_BUDGET,
            crate::synthetic::RESIDENT_POINT_BUDGET,
            crate::synthetic::RESIDENT_BATCH_BUDGET,
        );
        let depth_cue = EyeDomeLighting::new(1.25, 1).map_err(ViewFailure::corpus_gpu)?;
        WgpuRenderer::new(
            &gpu.device,
            RendererConfig::new(wgpu::TextureFormat::Rgba8Unorm, limits)
                .with_eye_dome_lighting(depth_cue),
        )
        .map_err(ViewFailure::corpus_gpu)
    })
}

fn run_with_gpu_setup(
    command: &CorpusCommand,
    acquire_gpu: impl FnOnce() -> Result<CorpusGpu, ViewFailure>,
    prepare_renderer: impl FnOnce(&CorpusGpu) -> Result<WgpuRenderer, ViewFailure>,
) -> Result<(), Box<dyn Error>> {
    let (manifest, gpu, mut renderer) =
        prepare_corpus_runtime(command, acquire_gpu, prepare_renderer)?;
    run_prepared_corpus(command, manifest, gpu, &mut renderer)
}

fn prepare_corpus_runtime<G, R>(
    command: &CorpusCommand,
    acquire_gpu: impl FnOnce() -> Result<G, ViewFailure>,
    prepare_renderer: impl FnOnce(&G) -> Result<R, ViewFailure>,
) -> Result<(CorpusManifest, G, R), Box<dyn Error>> {
    let manifest = CorpusManifest::load(command.manifest())
        .map_err(|error| manifest_loading_failure(&error))?;
    let gpu = acquire_gpu()?;
    let renderer = prepare_renderer(&gpu)?;
    Ok((manifest, gpu, renderer))
}

fn run_prepared_corpus(
    command: &CorpusCommand,
    manifest: CorpusManifest,
    gpu: CorpusGpu,
    renderer: &mut WgpuRenderer,
) -> Result<(), Box<dyn Error>> {
    let distinct_project_count = manifest.distinct_project_count();
    let distinct_firm_count = manifest.distinct_firm_count();
    let display_projection_matrix_complete = manifest.display_projection_matrix_complete();
    let CorpusManifest {
        schema: _,
        corpus_id,
        machine,
        entries,
        pre_v0_13_qualification,
        settlement_frame_ceiling,
    } = manifest;
    let mut reports = Vec::new();
    reports.try_reserve_exact(entries.len()).map_err(|error| {
        ViewFailure::resource(
            ViewPhase::HostStaging,
            format_args!("could not reserve corpus reports: {error}"),
        )
    })?;
    let mut first_failure = None;
    for (entry_index, entry) in entries.into_iter().enumerate() {
        let outcome = run_entry(
            entry,
            &gpu,
            renderer,
            corpus_view_generation(entry_index),
            pre_v0_13_qualification,
            settlement_frame_ceiling,
        );
        if first_failure.is_none() {
            first_failure = outcome.failure;
        }
        reports.push(outcome.report);
    }
    let summary = report_summary(
        &reports,
        distinct_project_count,
        distinct_firm_count,
        pre_v0_13_qualification,
        settlement_frame_ceiling,
        display_projection_matrix_complete,
    );
    let observed_depth_cue_status = depth_cue_status_name(renderer.depth_cue_status());
    let CorpusGpu {
        device: _,
        queue: _,
        name,
        backend,
    } = gpu;
    let machine = ReportMachine {
        declared_label: machine.label,
        declared_operating_system: machine.operating_system,
        declared_filesystem: machine.filesystem,
        declared_gpu_expectation: machine.gpu_expectation,
        observed_gpu_adapter: name,
        observed_gpu_backend: backend,
        observed_depth_cue_status,
    };
    let report = ViewingReport::new(corpus_id, machine, summary, reports);
    let bytes = report
        .encode()
        .map_err(|error| report_encoding_failure(&error).for_completed_corpus())?;
    let disposition = publish_report(command.report(), &bytes)
        .map_err(|error| report_publication_failure(&error).for_completed_corpus())?;
    println!(
        "Viewing report {:?}\n  schema: {REPORT_SCHEMA}\n  entries: {}\n  passed: {}\n  failed: {}\n  resource-limited: {}\n  bytes: {}",
        disposition,
        summary.entry_count,
        summary.passed_count,
        summary.failed_count,
        summary.resource_limited_count,
        bytes.len(),
    );
    if let Some(failure) = first_failure {
        return Err(Box::new(failure.for_completed_corpus()));
    }
    Ok(())
}

fn manifest_loading_failure(error: &io::Error) -> ViewFailure {
    if error.kind() == io::ErrorKind::OutOfMemory {
        ViewFailure::resource(
            ViewPhase::RequestValidation,
            format_args!("corpus manifest could not be accepted: {error}"),
        )
    } else {
        ViewFailure::invalid_request(format_args!(
            "corpus manifest could not be accepted: {error}"
        ))
    }
}

fn report_encoding_failure(error: &io::Error) -> ViewFailure {
    if error.kind() == io::ErrorKind::OutOfMemory {
        ViewFailure::resource(
            ViewPhase::ReportPublication,
            format_args!("viewing report could not be encoded: {error}"),
        )
    } else {
        ViewFailure::internal(
            ViewPhase::ReportPublication,
            format_args!("viewing report could not be encoded: {error}"),
        )
    }
}

fn report_publication_failure(error: &ReportPublicationError) -> ViewFailure {
    let (code, action) = match error {
        ReportPublicationError::Resource(_) => (
            ViewFailureCode::ResourceLimit,
            crate::diagnostic::RecoveryAction::RaiseNamedLimit,
        ),
        ReportPublicationError::Conflict(_) | ReportPublicationError::InvalidTarget(_) => (
            ViewFailureCode::InvalidRequest,
            crate::diagnostic::RecoveryAction::ChooseFreshCorpusTargets,
        ),
        ReportPublicationError::Io(_) => (
            ViewFailureCode::Io,
            crate::diagnostic::RecoveryAction::RestoreDisk,
        ),
    };
    ViewFailure::new(
        code,
        ViewPhase::ReportPublication,
        format_args!("viewing report could not be published: {error}"),
        action,
    )
}

#[derive(Debug)]
enum ReportPublicationError {
    Resource(io::Error),
    Conflict(io::Error),
    InvalidTarget(io::Error),
    Io(io::Error),
}

impl ReportPublicationError {
    fn classify(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::OutOfMemory => Self::Resource(error),
            io::ErrorKind::AlreadyExists => Self::Conflict(error),
            io::ErrorKind::InvalidData
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::IsADirectory
            | io::ErrorKind::NotADirectory
            | io::ErrorKind::NotFound => Self::InvalidTarget(error),
            _ => Self::Io(error),
        }
    }

    fn resource() -> Self {
        Self::Resource(out_of_memory())
    }
}

impl fmt::Display for ReportPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource(error)
            | Self::Conflict(error)
            | Self::InvalidTarget(error)
            | Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for ReportPublicationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Resource(error)
            | Self::Conflict(error)
            | Self::InvalidTarget(error)
            | Self::Io(error) => Some(error),
        }
    }
}

struct EntryRun {
    report: EntryReport,
    failure: Option<ViewFailure>,
}

#[derive(Clone, Copy)]
struct QualificationRun {
    enabled: bool,
    settlement_frame_ceiling: u32,
}

struct EntryRuntime<'run> {
    gpu: &'run CorpusGpu,
    renderer: &'run mut WgpuRenderer,
    view_generation: ViewGenerationKey,
    qualification: QualificationRun,
}

fn corpus_view_generation(entry_index: usize) -> ViewGenerationKey {
    let entry_offset = u64::try_from(entry_index).unwrap_or(u64::MAX);
    ViewGenerationKey::new(
        VIEW_GENERATION.view(),
        VIEW_GENERATION.generation().saturating_add(entry_offset),
    )
}

fn run_entry(
    entry: CorpusEntry,
    gpu: &CorpusGpu,
    renderer: &mut WgpuRenderer,
    view_generation: ViewGenerationKey,
    pre_v0_13_qualification: bool,
    settlement_frame_ceiling: u32,
) -> EntryRun {
    let mut evidence = EntryEvidence::new(&entry);
    let mut runtime = EntryRuntime {
        gpu,
        renderer,
        view_generation,
        qualification: QualificationRun {
            enabled: pre_v0_13_qualification,
            settlement_frame_ceiling,
        },
    };
    let failure = execute_entry(&entry, &mut runtime, &mut evidence).err();
    let report = evidence.finish(entry, failure.as_ref());
    EntryRun { report, failure }
}

fn execute_entry(
    entry: &CorpusEntry,
    runtime: &mut EntryRuntime<'_>,
    evidence: &mut EntryEvidence,
) -> Result<(), ViewFailure> {
    let verification_started = Instant::now();
    // Full verification retains the exact opened bytes, so this one operation
    // owns both Source identity and every fact recorded below.
    let source = source_las::open(entry.source_path())
        .blocking_wait()
        .map_err(|error| ViewFailure::source(entry.source_path(), &error))?;
    let source_verification_nanoseconds = elapsed_nanoseconds(verification_started.elapsed());
    evidence.record_source(
        source.identity(),
        source.metadata().point_count(),
        source.metadata().format_name(),
        source_verification_nanoseconds,
    )?;
    let display_mode = display_mode(entry.display());
    validate_display_source(display_mode, source.metadata())?;
    let recipe = display_mode.index_policy().recipe();
    let warm_source = source.clone();
    let prepare_started = Instant::now();
    let prepared =
        prepare_fresh_with_recipe(source, entry.index_path(), recipe, PrepareLimits::default())
            .blocking_wait()
            .map_err(|error| ViewFailure::index(entry.index_path(), &error))?;
    let index_prepare_nanoseconds = elapsed_nanoseconds(prepare_started.elapsed());
    evidence.record_index(&prepared, index_prepare_nanoseconds);
    require_cold_build(prepared.prepare_report().disposition())?;
    let warm_open_started = Instant::now();
    let warm = prepare_with_recipe(
        warm_source,
        entry.index_path(),
        recipe,
        PrepareLimits::default(),
    )
    .blocking_wait()
    .map_err(|error| ViewFailure::index(entry.index_path(), &error))?;
    let warm_open_nanoseconds = elapsed_nanoseconds(warm_open_started.elapsed());
    if warm.prepare_report().disposition() != PrepareDisposition::Opened {
        return Err(ViewFailure::internal(
            ViewPhase::IndexPrepare,
            "the immediate warm index measurement did not open the complete artifact",
        ));
    }
    evidence.record_warm_open(warm_open_nanoseconds);
    drop(prepared);

    run_prepared_entry(entry, warm, display_mode, runtime, evidence)
}

fn require_cold_build(disposition: PrepareDisposition) -> Result<(), ViewFailure> {
    if disposition == PrepareDisposition::Built {
        Ok(())
    } else {
        Err(ViewFailure::new(
            ViewFailureCode::InvalidRequest,
            ViewPhase::IndexPrepare,
            format_args!(
                "a corpus entry requires a fresh cold-build index target; observed {disposition:?}"
            ),
            crate::diagnostic::RecoveryAction::RebuildIndexExplicitly,
        ))
    }
}

fn run_prepared_entry(
    entry: &CorpusEntry,
    prepared: PreparedIndex,
    display_mode: DisplayMode,
    runtime: &mut EntryRuntime<'_>,
    evidence: &mut EntryEvidence,
) -> Result<(), ViewFailure> {
    let view_started = Instant::now();
    let real_scene = RealCloudScene::new(runtime.view_generation, prepared, display_mode)
        .map_err(|error| preserve_scene_failure(error, ViewPhase::Hierarchy, "hierarchy setup"))?;
    let mut scene = Scene::real(real_scene);
    evidence.observe_scene(scene.metrics());
    let projection = projection_mode(entry.projection());
    let mut camera =
        OrbitCamera::new(scene.camera_target(), scene.camera_radius()).with_projection(projection);
    let mut planner = ViewPlanner::new(
        PlannerConfig::new(2.0, 0.25)
            .map_err(|error| ViewFailure::internal(ViewPhase::Planning, error))?,
    );
    let mut density_transitions = DensityTransitions::default();
    let reset = runtime
        .renderer
        .apply(&RenderUpdate::Reset {
            view_generation: runtime.view_generation,
        })
        .map_err(|error| classify_renderer_failure(ViewPhase::GpuUpload, error))?;
    evidence.observe_resident(reset.resident());

    evidence
        .trace
        .try_reserve_exact(entry.trace().len().saturating_add(1))
        .map_err(|error| {
            ViewFailure::resource(
                ViewPhase::HostStaging,
                format_args!("could not reserve trace facts: {error}"),
            )
        })?;
    let initial = TraceStep::stationary(entry.initial_frame_count());
    let initial_report = run_trace_step(
        0,
        initial,
        TraceRuntime {
            scene: &mut scene,
            camera: &mut camera,
            planner: &mut planner,
            renderer: &mut *runtime.renderer,
            gpu: runtime.gpu,
            view_generation: runtime.view_generation,
            evidence,
            view_started: &view_started,
            density_transitions: &mut density_transitions,
        },
        runtime.qualification.enabled,
        runtime.qualification.settlement_frame_ceiling,
    )?;
    evidence.trace.push(initial_report);
    for (index, step) in entry.trace().iter().copied().enumerate() {
        camera.orbit(step.orbit_horizontal_pixels, step.orbit_vertical_pixels);
        camera
            .pan(
                step.pan_horizontal_pixels,
                step.pan_vertical_pixels,
                CORPUS_VIEWPORT[1],
            )
            .map_err(|error| ViewFailure::internal(ViewPhase::Planning, error))?;
        camera.zoom(step.zoom_lines);
        let report = run_trace_step(
            u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1),
            step,
            TraceRuntime {
                scene: &mut scene,
                camera: &mut camera,
                planner: &mut planner,
                renderer: &mut *runtime.renderer,
                gpu: runtime.gpu,
                view_generation: runtime.view_generation,
                evidence,
                view_started: &view_started,
                density_transitions: &mut density_transitions,
            },
            runtime.qualification.enabled,
            runtime.qualification.settlement_frame_ceiling,
        )?;
        evidence.trace.push(report);
    }
    evidence.observe_scene(scene.metrics());
    Ok(())
}

const CORPUS_VIEWPORT: [u32; 2] = [640, 480];

fn run_trace_step(
    step: u64,
    input: TraceStep,
    mut runtime: TraceRuntime<'_>,
    pre_v0_13_qualification: bool,
    settlement_frame_ceiling: u32,
) -> Result<TraceReport, ViewFailure> {
    if pre_v0_13_qualification {
        return run_settled_trace_step(step, input, runtime, settlement_frame_ceiling);
    }
    let mut pose = PoseEvidence::default();
    for _ in 0..input.frame_count {
        let frame = run_trace_frame(&mut runtime);
        runtime.evidence.observe_scene(runtime.scene.metrics());
        pose.observe(frame?);
    }
    let metrics = runtime.scene.metrics();
    Ok(pose.finish(step, input, metrics, SettlementEvidence::default()))
}

fn run_settled_trace_step(
    step: u64,
    input: TraceStep,
    mut runtime: TraceRuntime<'_>,
    settlement_frame_ceiling: u32,
) -> Result<TraceReport, ViewFailure> {
    let mut pose = PoseEvidence::default();
    let mut settlement = None;
    for frame_index in 1..=settlement_frame_ceiling {
        let frame = run_trace_frame(&mut runtime)?;
        runtime.evidence.observe_scene(runtime.scene.metrics());
        pose.observe(frame);
        if frame.is_quiet() && runtime.scene.metrics().queued_batches == 0 {
            settlement = Some((frame_index, runtime.view_started.elapsed()));
            break;
        }
    }
    let Some((settlement_frame, settlement_time)) = settlement else {
        return Err(ViewFailure::internal(
            ViewPhase::Rendering,
            format_args!(
                "corpus trace step {step} did not settle within {settlement_frame_ceiling} frames"
            ),
        ));
    };
    let settled_metrics = runtime.scene.metrics();
    let settled_nodes = runtime.scene.planning_nodes().as_slice().to_vec();
    for observation_frame in 1..=SETTLED_OBSERVATION_FRAMES {
        let frame = run_trace_frame(&mut runtime)?;
        runtime.evidence.observe_scene(runtime.scene.metrics());
        pose.observe(frame);
        if !frame.is_quiet()
            || runtime.scene.metrics() != settled_metrics
            || runtime.scene.planning_nodes().as_slice() != settled_nodes
        {
            return Err(ViewFailure::internal(
                ViewPhase::Rendering,
                format_args!(
                    "corpus trace step {step} changed during quiet observation frame {observation_frame}"
                ),
            ));
        }
    }
    Ok(pose.finish(
        step,
        input,
        runtime.scene.metrics(),
        SettlementEvidence {
            settlement_frame: Some(u64::from(settlement_frame)),
            settlement_nanoseconds: Some(elapsed_nanoseconds(settlement_time)),
            quiet_observation_frames: u64::from(SETTLED_OBSERVATION_FRAMES),
            quiet_window_complete: true,
        },
    ))
}

struct TraceRuntime<'run> {
    scene: &'run mut Scene,
    camera: &'run mut OrbitCamera,
    planner: &'run mut ViewPlanner,
    renderer: &'run mut WgpuRenderer,
    gpu: &'run CorpusGpu,
    view_generation: ViewGenerationKey,
    evidence: &'run mut EntryEvidence,
    view_started: &'run Instant,
    density_transitions: &'run mut DensityTransitions,
}

fn run_trace_frame(runtime: &mut TraceRuntime<'_>) -> Result<FrameEvidence, ViewFailure> {
    let viewport = Viewport::new(CORPUS_VIEWPORT[0], CORPUS_VIEWPORT[1])
        .map_err(|error| ViewFailure::internal(ViewPhase::Planning, error))?;
    let render_camera = runtime
        .camera
        .as_render_camera()
        .map_err(|error| ViewFailure::internal(ViewPhase::Planning, error))?;
    let (hierarchy, plan) = {
        let nodes = runtime.scene.planning_nodes();
        let hierarchy = nodes.as_slice().to_vec();
        let plan = runtime
            .planner
            .plan(
                &render_camera,
                viewport,
                AvailableNodes::new(runtime.view_generation, nodes.as_slice()),
                PLANNING_BUDGET,
            )
            .map_err(|error| ViewFailure::internal(ViewPhase::Planning, error))?;
        (hierarchy, plan)
    };
    let transition_actions = runtime.density_transitions.reconcile(&hierarchy, &plan);
    let mut transition_activity = apply_transition_actions(runtime, transition_actions)?;
    let issued = runtime
        .scene
        .reconcile_requests(plan.demanded_nodes(), plan.requests())
        .map_err(|error| {
            preserve_scene_failure(error, ViewPhase::Planning, "request reconciliation")
        })?;
    let mut accepted_batch = false;
    if let Some(batch) = runtime
        .scene
        .next_batch()
        .map_err(|error| preserve_scene_failure(error, ViewPhase::NodeRead, "node read"))?
    {
        let key = batch.key();
        let version = batch.version();
        match runtime.renderer.apply(&RenderUpdate::Upsert { batch }) {
            Ok(update) => {
                runtime.evidence.observe_resident(update.resident());
                runtime.evidence.observe_upload(update);
                runtime.scene.mark_resident(key, version);
                accepted_batch = true;
            }
            Err(error) => {
                runtime.scene.mark_rejected(key, version);
                return Err(classify_renderer_failure(ViewPhase::GpuUpload, error));
            }
        }
    }
    let submitted = std::time::Instant::now();
    let point_size =
        projected_spacing_point_size(viewport, runtime.scene.metrics().resident_points);
    let default_style = PointStyle::default();
    let reference_style = PointStyle::new(
        REFERENCE_POINT_SIZE_PIXELS,
        default_style.highlight_color(),
        default_style.clear_color(),
    )
    .map_err(|error| ViewFailure::internal(ViewPhase::Rendering, error))?;
    let style = reference_style
        .with_display_size_pixels(point_size)
        .map_err(|error| ViewFailure::internal(ViewPhase::Rendering, error))?;
    let frame_report = render_offscreen(
        runtime.renderer,
        runtime.gpu,
        runtime.view_generation,
        render_camera,
        viewport,
        style,
    )?;
    let completed_transition = runtime.density_transitions.advance_presented_frame();
    transition_activity.add(apply_transition_actions(runtime, completed_transition)?);
    if frame_report.drawn_points() > 0 {
        runtime
            .evidence
            .latch_first_visible_frame(runtime.view_started.elapsed());
    }
    runtime.evidence.observe_frame(frame_report);
    Ok(FrameEvidence {
        demanded_nodes: u64::try_from(plan.demanded_nodes().len()).unwrap_or(u64::MAX),
        requested_nodes: u64::try_from(plan.requests().len()).unwrap_or(u64::MAX),
        issued_nodes: issued,
        retired_nodes: transition_activity.retired,
        presentation_updates: transition_activity.presentations,
        accepted_batch,
        frame_report,
        submitted_frame_nanoseconds: elapsed_nanoseconds(submitted.elapsed()),
    })
}

fn apply_transition_actions(
    runtime: &mut TraceRuntime<'_>,
    actions: Vec<TransitionAction>,
) -> Result<TransitionActivity, ViewFailure> {
    let mut activity = TransitionActivity::default();
    for action in actions {
        let retiring = match action {
            TransitionAction::Present { batch, weight } => {
                let report = runtime
                    .renderer
                    .apply(&RenderUpdate::SetBatchPresentation {
                        view_generation: batch.view_generation,
                        key: batch.key,
                        expected_version: batch.expected_version,
                        weight,
                    })
                    .map_err(|error| classify_renderer_failure(ViewPhase::GpuUpload, error))?;
                runtime.evidence.observe_resident(report.resident());
                activity.presentations = activity.presentations.saturating_add(1);
                None
            }
            TransitionAction::Retire(batch) => {
                let report = runtime
                    .renderer
                    .apply(&RenderUpdate::Remove {
                        view_generation: batch.view_generation,
                        key: batch.key,
                        expected_version: batch.expected_version,
                    })
                    .map_err(|error| classify_renderer_failure(ViewPhase::GpuUpload, error))?;
                runtime.evidence.observe_resident(report.resident());
                activity.retired = activity.retired.saturating_add(1);
                Some(batch)
            }
        };
        if let Some(batch) = retiring {
            runtime
                .scene
                .mark_retired(batch.key, batch.expected_version);
        }
    }
    Ok(activity)
}

#[derive(Clone, Copy, Debug, Default)]
struct TransitionActivity {
    presentations: u64,
    retired: u64,
}

impl TransitionActivity {
    fn add(&mut self, other: Self) {
        self.presentations = self.presentations.saturating_add(other.presentations);
        self.retired = self.retired.saturating_add(other.retired);
    }
}

#[derive(Clone, Copy, Debug)]
struct FrameEvidence {
    demanded_nodes: u64,
    requested_nodes: u64,
    issued_nodes: u64,
    retired_nodes: u64,
    presentation_updates: u64,
    accepted_batch: bool,
    frame_report: render_wgpu::FrameReport,
    submitted_frame_nanoseconds: u64,
}

impl FrameEvidence {
    const fn is_quiet(self) -> bool {
        self.demanded_nodes == 0
            && self.requested_nodes == 0
            && self.issued_nodes == 0
            && self.retired_nodes == 0
            && self.presentation_updates == 0
            && !self.accepted_batch
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PoseEvidence {
    completed_frames: u64,
    peak_demanded_nodes: u64,
    peak_issued_nodes_per_frame: u64,
    issued_nodes: u64,
    retired_nodes: u64,
    presentation_updates: u64,
    accepted_batches: u64,
    final_drawn_points: u64,
    peak_resident_batches: u64,
    peak_resident_points: u64,
    peak_resident_bytes: u64,
    peak_transient_texture_bytes: u64,
    submitted_frame_nanoseconds: u64,
}

impl PoseEvidence {
    fn observe(&mut self, frame: FrameEvidence) {
        self.completed_frames = self.completed_frames.saturating_add(1);
        self.peak_demanded_nodes = self.peak_demanded_nodes.max(frame.demanded_nodes);
        self.peak_issued_nodes_per_frame = self.peak_issued_nodes_per_frame.max(frame.issued_nodes);
        self.issued_nodes = self.issued_nodes.saturating_add(frame.issued_nodes);
        self.retired_nodes = self.retired_nodes.saturating_add(frame.retired_nodes);
        self.presentation_updates = self
            .presentation_updates
            .saturating_add(frame.presentation_updates);
        self.accepted_batches = self
            .accepted_batches
            .saturating_add(u64::from(frame.accepted_batch));
        self.final_drawn_points = frame.frame_report.drawn_points();
        self.peak_resident_batches = self
            .peak_resident_batches
            .max(frame.frame_report.draw_calls());
        self.peak_resident_points = self
            .peak_resident_points
            .max(frame.frame_report.drawn_points());
        self.peak_resident_bytes = self
            .peak_resident_bytes
            .max(frame.frame_report.resident_bytes());
        self.peak_transient_texture_bytes = self
            .peak_transient_texture_bytes
            .max(frame.frame_report.transient_texture_bytes());
        self.submitted_frame_nanoseconds = self
            .submitted_frame_nanoseconds
            .saturating_add(frame.submitted_frame_nanoseconds);
    }

    fn finish(
        self,
        step: u64,
        input: TraceStep,
        metrics: SceneMetrics,
        settlement: SettlementEvidence,
    ) -> TraceReport {
        TraceReport {
            step,
            input,
            requested_frame_count: u64::from(input.frame_count),
            completed_frame_count: self.completed_frames,
            peak_demanded_nodes: self.peak_demanded_nodes,
            peak_issued_nodes_per_frame: self.peak_issued_nodes_per_frame,
            issued_nodes: self.issued_nodes,
            retired_nodes: self.retired_nodes,
            presentation_updates: self.presentation_updates,
            accepted_batches: self.accepted_batches,
            resident_batches: metrics.resident_batches,
            resident_points: metrics.resident_points,
            drawn_points: self.final_drawn_points,
            peak_resident_batches: self.peak_resident_batches,
            peak_resident_points: self.peak_resident_points,
            peak_resident_bytes: self.peak_resident_bytes,
            peak_transient_texture_bytes: self.peak_transient_texture_bytes,
            submitted_frame_nanoseconds: self.submitted_frame_nanoseconds,
            settlement_frame: settlement.settlement_frame,
            settlement_nanoseconds: settlement.settlement_nanoseconds,
            quiet_observation_frames: settlement.quiet_observation_frames,
            quiet_window_complete: settlement.quiet_window_complete,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SettlementEvidence {
    settlement_frame: Option<u64>,
    settlement_nanoseconds: Option<u64>,
    quiet_observation_frames: u64,
    quiet_window_complete: bool,
}

fn preserve_scene_failure(
    error: Box<dyn std::error::Error>,
    phase: ViewPhase,
    context: &'static str,
) -> ViewFailure {
    match error.downcast::<ViewFailure>() {
        Ok(failure) => *failure,
        Err(error) => ViewFailure::internal(phase, format_args!("{context} failed: {error}")),
    }
}

fn render_offscreen(
    renderer: &mut WgpuRenderer,
    gpu: &CorpusGpu,
    view_generation: ViewGenerationKey,
    camera: render_wgpu::Camera,
    viewport: Viewport,
    style: PointStyle,
) -> Result<render_wgpu::FrameReport, ViewFailure> {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("punctra corpus viewing target"),
        size: wgpu::Extent3d {
            width: CORPUS_VIEWPORT[0],
            height: CORPUS_VIEWPORT[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("punctra corpus viewing encoder"),
        });
    let frame = Frame::new(view_generation, camera, viewport)
        .map_err(|error| ViewFailure::internal(ViewPhase::Rendering, error))?
        .with_style(style);
    let recorded = renderer
        .render(&mut encoder, &target, &frame)
        .map_err(|error| classify_renderer_failure(ViewPhase::Rendering, error))?;
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| ViewFailure::gpu(ViewPhase::Rendering, error))?;
    Ok(recorded.report())
}

struct CorpusGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    backend: &'static str,
}

impl CorpusGpu {
    async fn new() -> Result<Self, ViewFailure> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(ViewFailure::corpus_gpu)?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("punctra corpus runner device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(ViewFailure::corpus_gpu)?;
        Ok(Self {
            device,
            queue,
            name: info.name,
            backend: gpu_backend_name(info.backend),
        })
    }
}

const fn gpu_backend_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Noop => "Noop",
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "Dx12",
        wgpu::Backend::Gl => "Gl",
        wgpu::Backend::BrowserWebGpu => "BrowserWebGpu",
    }
}

const fn depth_cue_status_name(status: DepthCueStatus) -> &'static str {
    match status {
        DepthCueStatus::Disabled => "disabled",
        DepthCueStatus::Active => "active",
        DepthCueStatus::UnsupportedFallback => "unsupported_fallback",
    }
}

const fn display_mode(choice: DisplayChoice) -> DisplayMode {
    match choice {
        DisplayChoice::Neutral => DisplayMode::Neutral,
        DisplayChoice::Elevation => DisplayMode::Elevation,
        DisplayChoice::Rgb => DisplayMode::Rgb,
        DisplayChoice::Intensity => DisplayMode::Intensity,
        DisplayChoice::Classification => DisplayMode::Classification,
    }
}

fn validate_display_source(
    mode: DisplayMode,
    metadata: &point_contracts::SourceMetadata,
) -> Result<(), ViewFailure> {
    mode.validate_source(metadata).map_err(|message| {
        ViewFailure::invalid_request(format_args!("corpus entry display {mode}: {message}"))
    })
}

const fn projection_mode(choice: ProjectionChoice) -> ProjectionMode {
    match choice {
        ProjectionChoice::Perspective => ProjectionMode::Perspective,
        ProjectionChoice::Orthographic => ProjectionMode::Orthographic,
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct SourceEvidence {
    source_identity: Option<String>,
    source_point_count: Option<u64>,
    source_format: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct IndexEvidence {
    index_recipe_version: u32,
    index_disk_version: u32,
    index_disposition: Option<&'static str>,
}

impl IndexEvidence {
    fn expected(display: DisplayChoice) -> Self {
        let (index_recipe_version, index_disk_version) =
            display_mode(display).index_policy().versions();
        Self {
            index_recipe_version,
            index_disk_version,
            index_disposition: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
pub(crate) struct MeasurementEvidence {
    source_verification_nanoseconds: Option<u64>,
    index_prepare_nanoseconds: Option<u64>,
    index_warm_open_nanoseconds: Option<u64>,
    first_accepted_visible_batch_nanoseconds: Option<u64>,
    index_artifact_bytes: Option<u64>,
    peak_index_temporary_disk_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
pub(crate) struct ResidencyEvidence {
    peak_queued_batches: u64,
    peak_queued_host_bytes: u64,
    peak_staged_points: u64,
    peak_staged_bytes: u64,
    resident_batches: u64,
    resident_points: u64,
    sampled_resident_points: u64,
    complete_resident_points: u64,
    retired_batches: u64,
    cancelled_requests: u64,
    rejected_batches: u64,
    cumulative_uploaded_batches: u64,
    cumulative_uploaded_points: u64,
    cumulative_uploaded_bytes: u64,
    peak_resident_batches: u64,
    peak_resident_points: u64,
    peak_resident_bytes: u64,
}

#[derive(Debug)]
struct EntryEvidence {
    source: SourceEvidence,
    index: IndexEvidence,
    measurements: MeasurementEvidence,
    residency: ResidencyEvidence,
    trace: Vec<TraceReport>,
}

impl EntryEvidence {
    fn new(entry: &CorpusEntry) -> Self {
        Self {
            source: SourceEvidence::default(),
            index: IndexEvidence::expected(entry.display()),
            measurements: MeasurementEvidence::default(),
            residency: ResidencyEvidence::default(),
            trace: Vec::new(),
        }
    }

    fn record_source(
        &mut self,
        identity: point_contracts::SourceId,
        point_count: u64,
        format: &str,
        verification_nanoseconds: u64,
    ) -> Result<(), ViewFailure> {
        self.source.source_identity = Some(source_identity_text(identity)?);
        self.source.source_point_count = Some(point_count);
        self.source.source_format = Some(owned_report_text(format)?);
        self.measurements.source_verification_nanoseconds = Some(verification_nanoseconds);
        Ok(())
    }

    fn record_index(&mut self, prepared: &PreparedIndex, prepare_nanoseconds: u64) {
        let descriptor = prepared.descriptor();
        let report = *prepared.prepare_report();
        self.index.index_recipe_version = descriptor.recipe_version();
        self.index.index_disk_version = descriptor.disk_version();
        self.index.index_disposition = Some(prepare_disposition(report.disposition()));
        self.measurements.index_prepare_nanoseconds = Some(prepare_nanoseconds);
        self.measurements.index_artifact_bytes = Some(report.artifact_bytes());
        self.measurements.peak_index_temporary_disk_bytes =
            Some(report.peak_temporary_disk_bytes());
    }

    fn record_warm_open(&mut self, warm_open_nanoseconds: u64) {
        self.measurements.index_warm_open_nanoseconds = Some(warm_open_nanoseconds);
    }

    fn latch_first_visible_frame(&mut self, elapsed: std::time::Duration) {
        if self
            .measurements
            .first_accepted_visible_batch_nanoseconds
            .is_none()
        {
            self.measurements.first_accepted_visible_batch_nanoseconds =
                Some(elapsed_nanoseconds(elapsed));
        }
    }

    fn observe_scene(&mut self, metrics: SceneMetrics) {
        self.residency.peak_queued_batches = self
            .residency
            .peak_queued_batches
            .max(metrics.peak_queued_batches);
        self.residency.peak_queued_host_bytes = self
            .residency
            .peak_queued_host_bytes
            .max(metrics.peak_queued_host_bytes);
        self.residency.peak_staged_points = self
            .residency
            .peak_staged_points
            .max(metrics.peak_staged_points);
        self.residency.peak_staged_bytes = self
            .residency
            .peak_staged_bytes
            .max(metrics.peak_staged_bytes);
        self.residency.resident_batches = metrics.resident_batches;
        self.residency.resident_points = metrics.resident_points;
        self.residency.sampled_resident_points = metrics.sampled_resident_points;
        self.residency.complete_resident_points = metrics.complete_resident_points;
        self.residency.retired_batches = metrics.retired_batches;
        self.residency.cancelled_requests = metrics.cancelled_requests;
        self.residency.rejected_batches = metrics.rejected_batches;
    }

    fn observe_upload(&mut self, report: render_protocol::UpdateReport) {
        if report.uploaded_points() > 0 {
            self.residency.cumulative_uploaded_batches =
                self.residency.cumulative_uploaded_batches.saturating_add(1);
        }
        self.residency.cumulative_uploaded_points = self
            .residency
            .cumulative_uploaded_points
            .saturating_add(report.uploaded_points());
        self.residency.cumulative_uploaded_bytes = self
            .residency
            .cumulative_uploaded_bytes
            .saturating_add(report.uploaded_bytes());
    }

    fn observe_resident(&mut self, resident: ResidentStats) {
        self.residency.peak_resident_batches = self
            .residency
            .peak_resident_batches
            .max(resident.batch_count());
        self.residency.peak_resident_points = self
            .residency
            .peak_resident_points
            .max(resident.point_count());
        self.residency.peak_resident_bytes = self
            .residency
            .peak_resident_bytes
            .max(resident.estimated_gpu_bytes());
    }

    fn observe_frame(&mut self, report: render_wgpu::FrameReport) {
        self.residency.peak_resident_batches = self
            .residency
            .peak_resident_batches
            .max(report.draw_calls());
        self.residency.peak_resident_points = self
            .residency
            .peak_resident_points
            .max(report.drawn_points());
        self.residency.peak_resident_bytes = self
            .residency
            .peak_resident_bytes
            .max(report.resident_bytes());
    }

    fn finish(self, entry: CorpusEntry, failure: Option<&ViewFailure>) -> EntryReport {
        let disposition = match failure.map(ViewFailure::code) {
            None => EntryDisposition::Passed,
            Some(ViewFailureCode::ResourceLimit) => EntryDisposition::ResourceLimited,
            Some(_) => EntryDisposition::Failed,
        };
        let CorpusEntry {
            id,
            project_id: _,
            firm_id: _,
            source_path: _,
            index_path: _,
            inspect_permission: _,
            measure_permission: _,
            display,
            projection,
            known_feature_outcomes,
            initial_frame_count,
            trace: declared_trace,
        } = entry;
        EntryReport {
            id,
            source: self.source,
            index: self.index,
            display,
            projection,
            known_feature_outcomes,
            declared_initial_frame_count: initial_frame_count,
            declared_trace,
            measurements: self.measurements,
            limits: EffectiveLimits::current(),
            residency: self.residency,
            trace: self.trace,
            disposition,
            failure: failure.map(|failure| FailureReport {
                code: failure.code().as_str(),
                phase: failure.phase().as_str(),
                message:
                    "entry did not complete; private local paths and adapter details are omitted",
                safe_action: ViewFailure::completed_corpus_action().as_str(),
            }),
        }
    }
}

fn source_identity_text(identity: point_contracts::SourceId) -> Result<String, ViewFailure> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::new();
    text.try_reserve_exact(64).map_err(|_| {
        ViewFailure::resource(
            ViewPhase::HostStaging,
            "could not reserve Source identity report text",
        )
    })?;
    for byte in identity.as_bytes() {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(text)
}

fn owned_report_text(text: &str) -> Result<String, ViewFailure> {
    let mut owned = String::new();
    owned.try_reserve_exact(text.len()).map_err(|_| {
        ViewFailure::resource(
            ViewPhase::HostStaging,
            "could not reserve corpus report text",
        )
    })?;
    owned.push_str(text);
    Ok(owned)
}

const fn prepare_disposition(disposition: PrepareDisposition) -> &'static str {
    match disposition {
        PrepareDisposition::Opened => "opened",
        PrepareDisposition::Built => "built",
        PrepareDisposition::Resumed => "resumed",
    }
}

fn report_summary(
    entries: &[EntryReport],
    distinct_project_count: u64,
    distinct_firm_count: u64,
    pre_v0_13_qualification: bool,
    settlement_frame_ceiling: u32,
    display_projection_matrix_complete: bool,
) -> ReportSummary {
    ReportSummary {
        entry_count: u64::try_from(entries.len()).unwrap_or(u64::MAX),
        distinct_project_count,
        distinct_firm_count,
        passed_count: u64::try_from(
            entries
                .iter()
                .filter(|entry| entry.disposition == EntryDisposition::Passed)
                .count(),
        )
        .unwrap_or(u64::MAX),
        failed_count: u64::try_from(
            entries
                .iter()
                .filter(|entry| entry.disposition == EntryDisposition::Failed)
                .count(),
        )
        .unwrap_or(u64::MAX),
        resource_limited_count: u64::try_from(
            entries
                .iter()
                .filter(|entry| entry.disposition == EntryDisposition::ResourceLimited)
                .count(),
        )
        .unwrap_or(u64::MAX),
        pre_v0_13_qualification,
        settlement_frame_ceiling: u64::from(settlement_frame_ceiling),
        quiet_observation_frames: if pre_v0_13_qualification {
            u64::from(SETTLED_OBSERVATION_FRAMES)
        } else {
            0
        },
        display_projection_matrix_complete,
        known_feature_located_count: known_feature_count(entries, KnownFeatureResult::Located),
        known_feature_issue_count: entries
            .iter()
            .flat_map(|entry| &entry.known_feature_outcomes)
            .filter(|outcome| outcome.result != KnownFeatureResult::Located)
            .count()
            .try_into()
            .unwrap_or(u64::MAX),
    }
}

fn known_feature_count(entries: &[EntryReport], result: KnownFeatureResult) -> u64 {
    u64::try_from(
        entries
            .iter()
            .flat_map(|entry| &entry.known_feature_outcomes)
            .filter(|outcome| outcome.result == result)
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn elapsed_nanoseconds(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn manifest_json_failure(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn deserialize_borrowed_entries<'de, D>(
    deserializer: D,
) -> Result<BoundedSequence<&'de serde_json::value::RawValue, MAX_ENTRIES>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_sequence(deserializer, MAX_ENTRIES, "corpus entries")
}

fn deserialize_borrowed_trace<'de, D>(
    deserializer: D,
) -> Result<BoundedSequence<TraceStep, MAX_TRACE_STEPS>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_sequence(deserializer, MAX_TRACE_STEPS, "navigation trace")
}

fn deserialize_known_feature_outcomes<'de, D>(
    deserializer: D,
) -> Result<BoundedSequence<KnownFeatureOutcome, MAX_KNOWN_FEATURE_OUTCOMES>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_sequence(
        deserializer,
        MAX_KNOWN_FEATURE_OUTCOMES,
        "known-feature outcomes",
    )
}

fn deserialize_bounded_sequence<'de, D, T, const MAXIMUM: usize>(
    deserializer: D,
    maximum: usize,
    role: &'static str,
) -> Result<BoundedSequence<T, MAXIMUM>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_seq(BoundedSequenceVisitor {
        maximum,
        role,
        item: PhantomData,
    })
}

#[derive(Debug)]
struct BoundedSequence<T, const MAXIMUM: usize> {
    values: [Option<T>; MAXIMUM],
    length: usize,
}

impl<T, const MAXIMUM: usize> BoundedSequence<T, MAXIMUM> {
    fn new() -> Self {
        Self {
            values: std::array::from_fn(|_| None),
            length: 0,
        }
    }

    const fn len(&self) -> usize {
        self.length
    }

    fn push(&mut self, value: T) {
        debug_assert!(self.length < MAXIMUM);
        self.values[self.length] = Some(value);
        self.length += 1;
    }

    fn into_values(self) -> impl Iterator<Item = T> {
        self.values
            .into_iter()
            .take(self.length)
            .map(|value| value.expect("a bounded sequence prefix is initialized"))
    }
}

impl<T, const MAXIMUM: usize> Default for BoundedSequence<T, MAXIMUM> {
    fn default() -> Self {
        Self::new()
    }
}

struct BoundedSequenceVisitor<T, const MAXIMUM: usize> {
    maximum: usize,
    role: &'static str,
    item: PhantomData<T>,
}

impl<'de, T, const MAXIMUM: usize> Visitor<'de> for BoundedSequenceVisitor<T, MAXIMUM>
where
    T: Deserialize<'de>,
{
    type Value = BoundedSequence<T, MAXIMUM>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} containing at most {} items",
            self.role, self.maximum
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let hinted = sequence.size_hint().unwrap_or(0);
        if hinted > self.maximum {
            return Err(A::Error::custom(format_args!(
                "{} exceeds {} items",
                self.role, self.maximum
            )));
        }
        let mut values = BoundedSequence::new();
        while values.len() < self.maximum {
            let Some(value) = sequence.next_element()? else {
                return Ok(values);
            };
            values.push(value);
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(A::Error::custom(format_args!(
                "{} exceeds {} items",
                self.role, self.maximum
            )));
        }
        Ok(values)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedCorpusManifest<'a> {
    #[serde(borrow)]
    schema: Cow<'a, str>,
    #[serde(borrow)]
    corpus_id: Cow<'a, str>,
    #[serde(borrow)]
    machine: BorrowedMachineDeclaration<'a>,
    #[serde(default)]
    pre_v0_13_qualification: bool,
    #[serde(default = "default_settlement_frame_ceiling")]
    settlement_frame_ceiling: u32,
    #[serde(borrow, deserialize_with = "deserialize_borrowed_entries")]
    entries: BoundedSequence<&'a serde_json::value::RawValue, MAX_ENTRIES>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedMachineDeclaration<'a> {
    #[serde(borrow)]
    label: Cow<'a, str>,
    #[serde(borrow)]
    operating_system: Cow<'a, str>,
    #[serde(borrow)]
    filesystem: Cow<'a, str>,
    #[serde(borrow)]
    gpu_expectation: Cow<'a, str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedCorpusEntry<'a> {
    #[serde(borrow)]
    id: Cow<'a, str>,
    #[serde(borrow)]
    project_id: Cow<'a, str>,
    #[serde(borrow)]
    firm_id: Cow<'a, str>,
    #[serde(borrow)]
    source_path: Cow<'a, str>,
    #[serde(borrow)]
    index_path: Cow<'a, str>,
    inspect_permission: bool,
    measure_permission: bool,
    display: DisplayChoice,
    projection: ProjectionChoice,
    #[serde(default, deserialize_with = "deserialize_known_feature_outcomes")]
    known_feature_outcomes: BoundedSequence<KnownFeatureOutcome, MAX_KNOWN_FEATURE_OUTCOMES>,
    #[serde(default = "default_frames_per_pose")]
    initial_frame_count: u32,
    #[serde(deserialize_with = "deserialize_borrowed_trace")]
    trace: BoundedSequence<TraceStep, MAX_TRACE_STEPS>,
}

impl BorrowedCorpusManifest<'_> {
    fn into_owned(
        self,
        permit_allocation: &mut impl FnMut() -> bool,
    ) -> io::Result<CorpusManifest> {
        let mut entries = manifest_vec(self.entries.len(), permit_allocation)?;
        for raw_entry in self.entries.into_values() {
            let entry: BorrowedCorpusEntry<'_> =
                serde_json::from_str(raw_entry.get()).map_err(manifest_json_failure)?;
            entries.push(entry.into_owned(permit_allocation)?);
        }
        Ok(CorpusManifest {
            schema: manifest_text(
                &self.schema,
                "corpus manifest schema",
                MANIFEST_SCHEMA.len(),
                permit_allocation,
            )?,
            corpus_id: manifest_text(
                &self.corpus_id,
                "corpus identifier",
                MAX_ID_BYTES,
                permit_allocation,
            )?,
            machine: self.machine.into_owned(permit_allocation)?,
            pre_v0_13_qualification: self.pre_v0_13_qualification,
            settlement_frame_ceiling: self.settlement_frame_ceiling,
            entries,
        })
    }
}

impl BorrowedMachineDeclaration<'_> {
    fn into_owned(
        self,
        permit_allocation: &mut impl FnMut() -> bool,
    ) -> io::Result<MachineDeclaration> {
        Ok(MachineDeclaration {
            label: manifest_text(
                &self.label,
                "machine declaration",
                MAX_TEXT_BYTES,
                permit_allocation,
            )?,
            operating_system: manifest_text(
                &self.operating_system,
                "machine declaration",
                MAX_TEXT_BYTES,
                permit_allocation,
            )?,
            filesystem: manifest_text(
                &self.filesystem,
                "machine declaration",
                MAX_TEXT_BYTES,
                permit_allocation,
            )?,
            gpu_expectation: manifest_text(
                &self.gpu_expectation,
                "machine declaration",
                MAX_TEXT_BYTES,
                permit_allocation,
            )?,
        })
    }
}

impl BorrowedCorpusEntry<'_> {
    fn into_owned(self, permit_allocation: &mut impl FnMut() -> bool) -> io::Result<CorpusEntry> {
        let mut trace = manifest_vec(self.trace.len(), permit_allocation)?;
        trace.extend(self.trace.into_values());
        Ok(CorpusEntry {
            id: manifest_text(
                &self.id,
                "corpus identifier",
                MAX_ID_BYTES,
                permit_allocation,
            )?,
            project_id: manifest_text(
                &self.project_id,
                "corpus identifier",
                MAX_ID_BYTES,
                permit_allocation,
            )?,
            firm_id: manifest_text(
                &self.firm_id,
                "corpus identifier",
                MAX_ID_BYTES,
                permit_allocation,
            )?,
            source_path: manifest_path(&self.source_path, permit_allocation)?,
            index_path: manifest_path(&self.index_path, permit_allocation)?,
            inspect_permission: self.inspect_permission,
            measure_permission: self.measure_permission,
            display: self.display,
            projection: self.projection,
            known_feature_outcomes: {
                let mut outcomes =
                    manifest_vec(self.known_feature_outcomes.len(), permit_allocation)?;
                outcomes.extend(self.known_feature_outcomes.into_values());
                outcomes
            },
            initial_frame_count: self.initial_frame_count,
            trace,
        })
    }
}

fn manifest_text(
    value: &str,
    role: &'static str,
    maximum: usize,
    permit_allocation: &mut impl FnMut() -> bool,
) -> io::Result<String> {
    validate_text(role, value, maximum)?;
    if !permit_allocation() {
        return Err(out_of_memory());
    }
    let mut owned = String::new();
    owned
        .try_reserve_exact(value.len())
        .map_err(|_| out_of_memory())?;
    if owned.capacity() > value.len() {
        return Err(out_of_memory());
    }
    owned.push_str(value);
    Ok(owned)
}

fn manifest_path(value: &str, permit_allocation: &mut impl FnMut() -> bool) -> io::Result<PathBuf> {
    validate_text("corpus path", value, MAX_PATH_BYTES)?;
    if !permit_allocation() {
        return Err(out_of_memory());
    }
    let mut path = PathBuf::new();
    path.try_reserve_exact(value.len())
        .map_err(|_| out_of_memory())?;
    if path.capacity() > value.len() {
        return Err(out_of_memory());
    }
    path.push(value);
    Ok(path)
}

fn manifest_vec<T>(
    length: usize,
    permit_allocation: &mut impl FnMut() -> bool,
) -> io::Result<Vec<T>> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if !permit_allocation() {
        return Err(out_of_memory());
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| out_of_memory())?;
    if values.capacity() > length {
        return Err(out_of_memory());
    }
    Ok(values)
}

fn parse_manifest_bytes(
    bytes: &[u8],
    permit_allocation: &mut impl FnMut() -> bool,
) -> io::Result<CorpusManifest> {
    preflight_manifest_json_strings(bytes)?;
    let borrowed: BorrowedCorpusManifest<'_> =
        serde_json::from_slice(bytes).map_err(manifest_json_failure)?;
    let manifest = borrowed.into_owned(permit_allocation)?;
    manifest.validate()?;
    Ok(manifest)
}

#[derive(Debug, PartialEq)]
pub(crate) struct CorpusManifest {
    schema: String,
    corpus_id: String,
    machine: MachineDeclaration,
    entries: Vec<CorpusEntry>,
    pre_v0_13_qualification: bool,
    settlement_frame_ceiling: u32,
}

impl CorpusManifest {
    pub(crate) fn load(path: &Path) -> io::Result<Self> {
        let (bytes, _) = read_regular_bounded(path, MAX_MANIFEST_BYTES, "corpus manifest")?;
        parse_manifest_bytes(&bytes, &mut || true)
    }

    fn validate(&self) -> io::Result<()> {
        if self.schema != MANIFEST_SCHEMA {
            return invalid(format!("corpus manifest schema must be {MANIFEST_SCHEMA}"));
        }
        validate_text("corpus_id", &self.corpus_id, MAX_ID_BYTES)?;
        self.machine.validate()?;
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRIES {
            return invalid(format!(
                "corpus entries must contain between 1 and {MAX_ENTRIES} items"
            ));
        }
        for (index, entry) in self.entries.iter().enumerate() {
            entry.validate()?;
            if self.entries[..index]
                .iter()
                .any(|previous| previous.id == entry.id)
            {
                return invalid("corpus entry IDs must be unique");
            }
        }
        if !(1..=MAX_SETTLEMENT_FRAME_CEILING).contains(&self.settlement_frame_ceiling) {
            return invalid(format!(
                "settlement_frame_ceiling must contain between 1 and {MAX_SETTLEMENT_FRAME_CEILING} frames"
            ));
        }
        if self.pre_v0_13_qualification {
            if self.distinct_project_count() < 5 || self.distinct_firm_count() < 3 {
                return invalid(
                    "pre-v0.13 qualification requires at least five projects from three unrelated firms",
                );
            }
            if !self.display_projection_matrix_complete() {
                return invalid(
                    "pre-v0.13 qualification requires all five display modes in both projections",
                );
            }
            if !self.known_feature_matrix_complete() {
                return invalid(
                    "pre-v0.13 qualification requires outcomes for every declared known-feature kind",
                );
            }
        }
        Ok(())
    }

    fn display_projection_matrix_complete(&self) -> bool {
        DisplayChoice::ALL.into_iter().all(|display| {
            ProjectionChoice::ALL.into_iter().all(|projection| {
                self.entries
                    .iter()
                    .any(|entry| entry.display == display && entry.projection == projection)
            })
        })
    }

    fn known_feature_matrix_complete(&self) -> bool {
        KnownFeatureKind::ALL.into_iter().all(|kind| {
            self.entries.iter().any(|entry| {
                entry
                    .known_feature_outcomes
                    .iter()
                    .any(|outcome| outcome.kind == kind)
            })
        })
    }

    pub(crate) fn distinct_project_count(&self) -> u64 {
        u64::try_from(
            self.entries
                .iter()
                .enumerate()
                .filter(|(index, entry)| {
                    !self.entries[..*index]
                        .iter()
                        .any(|previous| previous.project_id == entry.project_id)
                })
                .count(),
        )
        .unwrap_or(u64::MAX)
    }

    pub(crate) fn distinct_firm_count(&self) -> u64 {
        u64::try_from(
            self.entries
                .iter()
                .enumerate()
                .filter(|(index, entry)| {
                    !self.entries[..*index]
                        .iter()
                        .any(|previous| previous.firm_id == entry.firm_id)
                })
                .count(),
        )
        .unwrap_or(u64::MAX)
    }
}

fn preflight_manifest_json_strings(bytes: &[u8]) -> io::Result<()> {
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'"' {
            cursor += 1;
            continue;
        }

        cursor += 1;
        let string_start = cursor;
        let mut decoded_bytes = 0_usize;
        loop {
            let Some(&byte) = bytes.get(cursor) else {
                return invalid("corpus manifest contains an unterminated JSON string");
            };
            if cursor - string_start > MAX_MANIFEST_STRING_TOKEN_BYTES {
                return invalid(format!(
                    "corpus manifest encoded JSON strings may contain at most {MAX_MANIFEST_STRING_TOKEN_BYTES} bytes"
                ));
            }
            match byte {
                b'"' => {
                    cursor += 1;
                    break;
                }
                b'\\' => {
                    let (next, escaped_bytes) = preflight_json_escape(bytes, cursor)?;
                    cursor = next;
                    decoded_bytes = decoded_bytes.saturating_add(escaped_bytes);
                }
                0x00..=0x1f => {
                    return invalid("corpus manifest JSON strings may not contain control bytes");
                }
                _ => {
                    cursor += 1;
                    decoded_bytes = decoded_bytes.saturating_add(1);
                }
            }
            if decoded_bytes > MAX_PATH_BYTES {
                return invalid(format!(
                    "corpus manifest decoded JSON strings may contain at most {MAX_PATH_BYTES} UTF-8 bytes"
                ));
            }
        }
    }
    Ok(())
}

fn preflight_json_escape(bytes: &[u8], cursor: usize) -> io::Result<(usize, usize)> {
    let Some(&escaped) = bytes.get(cursor + 1) else {
        return invalid("corpus manifest contains an incomplete JSON escape");
    };
    if matches!(
        escaped,
        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't'
    ) {
        return Ok((cursor + 2, 1));
    }
    if escaped != b'u' {
        return invalid("corpus manifest contains an invalid JSON escape");
    }

    let scalar = json_hex_quad(bytes, cursor + 2)?;
    if (0xd800..=0xdbff).contains(&scalar) {
        if bytes.get(cursor + 6..cursor + 8) != Some(&b"\\u"[..]) {
            return invalid("corpus manifest contains an unpaired JSON high surrogate");
        }
        let low = json_hex_quad(bytes, cursor + 8)?;
        if !(0xdc00..=0xdfff).contains(&low) {
            return invalid("corpus manifest contains an invalid JSON surrogate pair");
        }
        return Ok((cursor + 12, 4));
    }
    if (0xdc00..=0xdfff).contains(&scalar) {
        return invalid("corpus manifest contains an unpaired JSON low surrogate");
    }
    let decoded = char::from_u32(u32::from(scalar))
        .expect("a non-surrogate JSON code unit is a Unicode scalar")
        .len_utf8();
    Ok((cursor + 6, decoded))
}

fn json_hex_quad(bytes: &[u8], start: usize) -> io::Result<u16> {
    let Some(digits) = bytes.get(start..start + 4) else {
        return invalid("corpus manifest contains an incomplete JSON Unicode escape");
    };
    let mut value = 0_u16;
    for digit in digits {
        let nibble = match digit {
            b'0'..=b'9' => u16::from(*digit - b'0'),
            b'a'..=b'f' => u16::from(*digit - b'a' + 10),
            b'A'..=b'F' => u16::from(*digit - b'A' + 10),
            _ => return invalid("corpus manifest contains an invalid JSON Unicode escape"),
        };
        value = value * 16 + nibble;
    }
    Ok(value)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct MachineDeclaration {
    label: String,
    operating_system: String,
    filesystem: String,
    gpu_expectation: String,
}

impl MachineDeclaration {
    fn validate(&self) -> io::Result<()> {
        validate_text("machine label", &self.label, MAX_TEXT_BYTES)?;
        validate_text(
            "machine operating_system",
            &self.operating_system,
            MAX_TEXT_BYTES,
        )?;
        validate_text("machine filesystem", &self.filesystem, MAX_TEXT_BYTES)?;
        validate_text(
            "machine gpu_expectation",
            &self.gpu_expectation,
            MAX_TEXT_BYTES,
        )
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct CorpusEntry {
    id: String,
    project_id: String,
    firm_id: String,
    source_path: PathBuf,
    index_path: PathBuf,
    inspect_permission: bool,
    measure_permission: bool,
    display: DisplayChoice,
    projection: ProjectionChoice,
    known_feature_outcomes: Vec<KnownFeatureOutcome>,
    initial_frame_count: u32,
    trace: Vec<TraceStep>,
}

impl CorpusEntry {
    fn validate(&self) -> io::Result<()> {
        validate_text("entry id", &self.id, MAX_ID_BYTES)?;
        validate_text("project_id", &self.project_id, MAX_ID_BYTES)?;
        validate_text("firm_id", &self.firm_id, MAX_ID_BYTES)?;
        validate_path("source_path", &self.source_path)?;
        validate_path("index_path", &self.index_path)?;
        if self.source_path == self.index_path {
            return invalid("Source and index paths must differ");
        }
        if !self.inspect_permission || !self.measure_permission {
            return invalid(format!(
                "entry {} requires explicit inspect_permission and measure_permission",
                self.id
            ));
        }
        if self.trace.len() > MAX_TRACE_STEPS {
            return invalid(format!(
                "entry {} trace exceeds {MAX_TRACE_STEPS} steps",
                self.id
            ));
        }
        if self.known_feature_outcomes.len() > MAX_KNOWN_FEATURE_OUTCOMES {
            return invalid(format!(
                "entry {} known_feature_outcomes exceeds {MAX_KNOWN_FEATURE_OUTCOMES} items",
                self.id
            ));
        }
        for (index, outcome) in self.known_feature_outcomes.iter().enumerate() {
            if self.known_feature_outcomes[..index]
                .iter()
                .any(|previous| previous.kind == outcome.kind)
            {
                return invalid(format!("entry {} repeats one known-feature kind", self.id));
            }
        }
        validate_frame_count(self.initial_frame_count)?;
        for step in &self.trace {
            step.validate()?;
        }
        Ok(())
    }

    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn index_path(&self) -> &Path {
        &self.index_path
    }

    pub(crate) const fn display(&self) -> DisplayChoice {
        self.display
    }

    pub(crate) const fn projection(&self) -> ProjectionChoice {
        self.projection
    }

    pub(crate) const fn initial_frame_count(&self) -> u32 {
        self.initial_frame_count
    }

    pub(crate) fn trace(&self) -> &[TraceStep] {
        &self.trace
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DisplayChoice {
    Neutral,
    Elevation,
    Rgb,
    Intensity,
    Classification,
}

impl DisplayChoice {
    const ALL: [Self; 5] = [
        Self::Neutral,
        Self::Elevation,
        Self::Rgb,
        Self::Intensity,
        Self::Classification,
    ];
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectionChoice {
    Perspective,
    Orthographic,
}

impl ProjectionChoice {
    const ALL: [Self; 2] = [Self::Perspective, Self::Orthographic];
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KnownFeatureKind {
    TerrainBreak,
    Vegetation,
    Building,
    ScanPatternVariation,
    LowIntensity,
    HighIntensity,
    RepresentativeClassification,
}

impl KnownFeatureKind {
    const ALL: [Self; 7] = [
        Self::TerrainBreak,
        Self::Vegetation,
        Self::Building,
        Self::ScanPatternVariation,
        Self::LowIntensity,
        Self::HighIntensity,
        Self::RepresentativeClassification,
    ];
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KnownFeatureResult {
    Located,
    ArtifactConfounded,
    NotObserved,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnownFeatureOutcome {
    kind: KnownFeatureKind,
    result: KnownFeatureResult,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceStep {
    pub(crate) orbit_horizontal_pixels: f64,
    pub(crate) orbit_vertical_pixels: f64,
    pub(crate) pan_horizontal_pixels: f64,
    pub(crate) pan_vertical_pixels: f64,
    pub(crate) zoom_lines: f64,
    #[serde(default = "default_frames_per_pose")]
    pub(crate) frame_count: u32,
}

impl TraceStep {
    const fn stationary(frame_count: u32) -> Self {
        Self {
            orbit_horizontal_pixels: 0.0,
            orbit_vertical_pixels: 0.0,
            pan_horizontal_pixels: 0.0,
            pan_vertical_pixels: 0.0,
            zoom_lines: 0.0,
            frame_count,
        }
    }

    fn validate(self) -> io::Result<()> {
        let values = [
            self.orbit_horizontal_pixels,
            self.orbit_vertical_pixels,
            self.pan_horizontal_pixels,
            self.pan_vertical_pixels,
            self.zoom_lines,
        ];
        if !values.into_iter().all(f64::is_finite) {
            return invalid("navigation trace values must be finite");
        }
        validate_frame_count(self.frame_count)
    }
}

const fn default_frames_per_pose() -> u32 {
    DEFAULT_FRAMES_PER_POSE
}

const fn default_settlement_frame_ceiling() -> u32 {
    DEFAULT_SETTLEMENT_FRAME_CEILING
}

fn validate_frame_count(frame_count: u32) -> io::Result<()> {
    if (1..=MAX_FRAMES_PER_POSE).contains(&frame_count) {
        Ok(())
    } else {
        invalid(format!(
            "frame_count must contain between 1 and {MAX_FRAMES_PER_POSE} frames"
        ))
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct ViewingReport {
    schema: &'static str,
    executable_version: &'static str,
    corpus_id: String,
    machine: ReportMachine,
    summary: ReportSummary,
    entries: Vec<EntryReport>,
    nonclaims: ReportNonclaims,
}

impl ViewingReport {
    pub(crate) fn new(
        corpus_id: String,
        machine: ReportMachine,
        summary: ReportSummary,
        entries: Vec<EntryReport>,
    ) -> Self {
        Self {
            schema: REPORT_SCHEMA,
            executable_version: env!("CARGO_PKG_VERSION"),
            corpus_id,
            machine,
            summary,
            entries,
            nonclaims: ReportNonclaims::default(),
        }
    }

    pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
        encode_report_bounded(self, MAX_REPORT_BYTES)
    }
}

fn encode_report_bounded(report: &ViewingReport, maximum: u64) -> io::Result<Vec<u8>> {
    let mut output = BoundedReportBuffer::new(maximum)?;
    serde_json::to_writer(&mut output, report).map_err(|error| {
        if let Some(kind) = error.io_error_kind() {
            io::Error::new(kind, error)
        } else {
            io::Error::other(error)
        }
    })?;
    output.write_all(b"\n")?;
    Ok(output.into_inner())
}

struct BoundedReportBuffer {
    bytes: Vec<u8>,
    maximum: u64,
}

impl BoundedReportBuffer {
    fn new(maximum: u64) -> io::Result<Self> {
        let capacity = usize::try_from(maximum).map_err(|_| out_of_memory())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| out_of_memory())?;
        if bytes.capacity() > capacity {
            return Err(out_of_memory());
        }
        Ok(Self { bytes, maximum })
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedReportBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let required = self
            .bytes
            .len()
            .checked_add(buffer.len())
            .ok_or_else(out_of_memory)?;
        if required as u64 > self.maximum {
            return Err(out_of_memory());
        }
        debug_assert!(required <= self.bytes.capacity());
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct ReportMachine {
    pub(crate) declared_label: String,
    pub(crate) declared_operating_system: String,
    pub(crate) declared_filesystem: String,
    pub(crate) declared_gpu_expectation: String,
    pub(crate) observed_gpu_adapter: String,
    pub(crate) observed_gpu_backend: &'static str,
    pub(crate) observed_depth_cue_status: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct ReportSummary {
    pub(crate) entry_count: u64,
    pub(crate) distinct_project_count: u64,
    pub(crate) distinct_firm_count: u64,
    pub(crate) passed_count: u64,
    pub(crate) failed_count: u64,
    pub(crate) resource_limited_count: u64,
    pub(crate) pre_v0_13_qualification: bool,
    pub(crate) settlement_frame_ceiling: u64,
    pub(crate) quiet_observation_frames: u64,
    pub(crate) display_projection_matrix_complete: bool,
    pub(crate) known_feature_located_count: u64,
    pub(crate) known_feature_issue_count: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct EntryReport {
    pub(crate) id: String,
    #[serde(flatten)]
    pub(crate) source: SourceEvidence,
    #[serde(flatten)]
    pub(crate) index: IndexEvidence,
    pub(crate) display: DisplayChoice,
    pub(crate) projection: ProjectionChoice,
    pub(crate) known_feature_outcomes: Vec<KnownFeatureOutcome>,
    pub(crate) declared_initial_frame_count: u32,
    pub(crate) declared_trace: Vec<TraceStep>,
    #[serde(flatten)]
    pub(crate) measurements: MeasurementEvidence,
    pub(crate) limits: EffectiveLimits,
    #[serde(flatten)]
    pub(crate) residency: ResidencyEvidence,
    pub(crate) trace: Vec<TraceReport>,
    pub(crate) disposition: EntryDisposition,
    pub(crate) failure: Option<FailureReport>,
}

#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryDisposition {
    Passed,
    Failed,
    ResourceLimited,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub(crate) struct TraceReport {
    pub(crate) step: u64,
    pub(crate) input: TraceStep,
    pub(crate) requested_frame_count: u64,
    pub(crate) completed_frame_count: u64,
    pub(crate) peak_demanded_nodes: u64,
    pub(crate) peak_issued_nodes_per_frame: u64,
    pub(crate) issued_nodes: u64,
    pub(crate) retired_nodes: u64,
    pub(crate) presentation_updates: u64,
    pub(crate) accepted_batches: u64,
    pub(crate) resident_batches: u64,
    pub(crate) resident_points: u64,
    pub(crate) drawn_points: u64,
    pub(crate) peak_resident_batches: u64,
    pub(crate) peak_resident_points: u64,
    pub(crate) peak_resident_bytes: u64,
    pub(crate) peak_transient_texture_bytes: u64,
    pub(crate) submitted_frame_nanoseconds: u64,
    pub(crate) settlement_frame: Option<u64>,
    pub(crate) settlement_nanoseconds: Option<u64>,
    pub(crate) quiet_observation_frames: u64,
    pub(crate) quiet_window_complete: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct EffectiveLimits {
    source_verification_working_bytes: u64,
    index_source_batch_points: u64,
    index_source_batch_payload_bytes: u64,
    index_adapter_working_bytes: u64,
    index_build_working_bytes: u64,
    index_incomplete_bytes: u64,
    index_artifact_bytes: u64,
    index_hierarchy_nodes: u64,
    index_resident_metadata_bytes: u64,
    node_emitted_points: u64,
    node_source_spans: u64,
    node_source_batch_points: u64,
    node_source_batch_payload_bytes: u64,
    node_display_batch_bytes: u64,
    node_index_buffer_bytes: u64,
    node_adapter_working_bytes: u64,
    planner_points: u64,
    planner_estimated_bytes: u64,
    planner_batches: u64,
    queue_nodes: u64,
    queue_host_bytes: u64,
    staging_points: u64,
    staging_bytes: u64,
    hierarchy_bytes: u64,
    renderer_points: u64,
    renderer_estimated_gpu_bytes: u64,
    renderer_batches: u64,
    manifest_bytes: u64,
    report_bytes: u64,
    manifest_entries: u64,
    trace_steps_per_entry: u64,
    frames_per_pose: u64,
    settlement_frame_ceiling: u64,
    settled_observation_frames: u64,
}

impl EffectiveLimits {
    fn current() -> Self {
        let prepare = PrepareLimits::default();
        let node = NodeReadBudget::new(STAGING_POINT_BUDGET, STAGING_BYTE_BUDGET)
            .expect("the static node-read limits are nonzero");
        Self {
            source_verification_working_bytes: SOURCE_VERIFICATION_WORKING_BYTES,
            index_source_batch_points: prepare.max_source_batch_points(),
            index_source_batch_payload_bytes: prepare.max_source_batch_payload_bytes(),
            index_adapter_working_bytes: prepare.max_adapter_working_bytes(),
            index_build_working_bytes: prepare.max_build_working_bytes(),
            index_incomplete_bytes: prepare.max_incomplete_bytes(),
            index_artifact_bytes: prepare.max_artifact_bytes(),
            index_hierarchy_nodes: prepare.max_hierarchy_nodes(),
            index_resident_metadata_bytes: prepare.max_resident_metadata_bytes(),
            node_emitted_points: node.max_emitted_points(),
            node_source_spans: node.max_source_spans(),
            node_source_batch_points: node.max_source_batch_points(),
            node_source_batch_payload_bytes: node.max_source_batch_payload_bytes(),
            node_display_batch_bytes: node.max_display_batch_bytes(),
            node_index_buffer_bytes: node.max_index_buffer_bytes(),
            node_adapter_working_bytes: node.max_adapter_working_bytes(),
            planner_points: PLANNING_BUDGET.max_points(),
            planner_estimated_bytes: PLANNING_BUDGET.max_estimated_bytes(),
            planner_batches: PLANNING_BUDGET.max_batches(),
            queue_nodes: QUEUED_NODE_BUDGET,
            queue_host_bytes: QUEUED_HOST_BYTE_BUDGET,
            staging_points: STAGING_POINT_BUDGET,
            staging_bytes: STAGING_BYTE_BUDGET,
            hierarchy_bytes: HIERARCHY_BYTE_BUDGET,
            renderer_points: crate::synthetic::RESIDENT_POINT_BUDGET,
            renderer_estimated_gpu_bytes: crate::synthetic::RESIDENT_BYTE_BUDGET,
            renderer_batches: crate::synthetic::RESIDENT_BATCH_BUDGET,
            manifest_bytes: MAX_MANIFEST_BYTES,
            report_bytes: MAX_REPORT_BYTES,
            manifest_entries: MAX_ENTRIES as u64,
            trace_steps_per_entry: MAX_TRACE_STEPS as u64,
            frames_per_pose: u64::from(MAX_FRAMES_PER_POSE),
            settlement_frame_ceiling: u64::from(MAX_SETTLEMENT_FRAME_CEILING),
            settled_observation_frames: u64::from(SETTLED_OBSERVATION_FRAMES),
        }
    }
}

#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub(crate) struct FailureReport {
    pub(crate) code: &'static str,
    pub(crate) phase: &'static str,
    pub(crate) message: &'static str,
    pub(crate) safe_action: &'static str,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
struct ReportNonclaims {
    production_corpus_complete: bool,
    partner_acceptance_evaluated: bool,
    professional_preference_evaluated: bool,
    terrain_resource_envelope_evaluated: bool,
    human_time_savings_evaluated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublishDisposition {
    Created,
    Reconciled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReportPublicationBoundary {
    BeforeLink,
    AfterLink,
    TerminalAcknowledgement,
}

trait ReportPublicationHook {
    fn reach(&self, boundary: ReportPublicationBoundary) -> io::Result<()>;
}

struct ProductionReportPublicationHook;

impl ReportPublicationHook for ProductionReportPublicationHook {
    fn reach(&self, _boundary: ReportPublicationBoundary) -> io::Result<()> {
        Ok(())
    }
}

struct BoundReportTarget {
    requested_parent: PathBuf,
    canonical_parent: PathBuf,
    target: PathBuf,
    parent_identity: fs::Metadata,
}

impl BoundReportTarget {
    fn bind(target: &Path) -> io::Result<Self> {
        let Some(Component::Normal(file_name)) = target.components().next_back() else {
            return invalid("viewing report target needs a normal file name");
        };
        let requested_parent = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let canonical_parent = fs::canonicalize(&requested_parent)?;
        let parent_identity = fs::symlink_metadata(&canonical_parent)?;
        if !parent_identity.file_type().is_dir() {
            return invalid("viewing report parent must resolve to a directory");
        }
        let binding = Self {
            requested_parent,
            target: canonical_parent.join(file_name),
            canonical_parent,
            parent_identity,
        };
        binding.verify()?;
        Ok(binding)
    }

    fn verify(&self) -> io::Result<()> {
        verify_directory(&self.canonical_parent, &self.parent_identity)?;
        if fs::canonicalize(&self.requested_parent)? == self.canonical_parent {
            Ok(())
        } else {
            invalid("viewing report parent binding changed")
        }
    }

    fn target(&self) -> &Path {
        &self.target
    }

    fn parent(&self) -> &Path {
        &self.canonical_parent
    }
}

fn publish_report(path: &Path, bytes: &[u8]) -> Result<PublishDisposition, ReportPublicationError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_REPORT_BYTES {
        return Err(ReportPublicationError::resource());
    }
    let binding = BoundReportTarget::bind(path).map_err(ReportPublicationError::classify)?;
    publish_bound_report(&binding, bytes, &ProductionReportPublicationHook)
        .map_err(ReportPublicationError::classify)
}

fn publish_bound_report(
    binding: &BoundReportTarget,
    bytes: &[u8],
    hook: &impl ReportPublicationHook,
) -> io::Result<PublishDisposition> {
    binding.verify()?;
    match fs::symlink_metadata(binding.target()) {
        Ok(_) => {
            let disposition = reconcile_existing(binding.target(), bytes)?;
            hook.reach(ReportPublicationBoundary::TerminalAcknowledgement)?;
            binding.verify()?;
            reconcile_existing(binding.target(), bytes)?;
            return Ok(disposition);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let parent = binding.parent();
    let (stage_path, mut stage) = create_stage(binding.target())?;
    let stage_identity = stage.metadata()?;
    let mut linked = false;
    let publish = (|| {
        stage.write_all(bytes)?;
        stage.sync_all()?;
        stage.seek(SeekFrom::Start(0))?;
        let stage_state = stage.metadata()?;
        let read_back = read_exact_file_bytes(&mut stage, bytes.len(), "viewing report stage")?;
        if !same_file_identity(&stage_identity, &stage_state)
            || !same_file_state(&stage_state, &stage.metadata()?)
        {
            return invalid("viewing report stage changed while verifying");
        }
        if read_back != bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "viewing report stage read-back differs",
            ));
        }
        binding.verify()?;
        hook.reach(ReportPublicationBoundary::BeforeLink)?;
        binding.verify()?;
        match fs::hard_link(&stage_path, binding.target()) {
            Ok(()) => {
                linked = true;
                hook.reach(ReportPublicationBoundary::AfterLink)?;
                binding.verify()?;
                let (_, linked_identity) = read_regular_bounded(
                    binding.target(),
                    MAX_REPORT_BYTES,
                    "viewing report target",
                )?;
                if !same_file_identity(&stage_identity, &linked_identity) {
                    return invalid("viewing report target changed after publication");
                }
                sync_directory(parent)?;
                let disposition = reconcile_existing(binding.target(), bytes)?;
                debug_assert_eq!(disposition, PublishDisposition::Reconciled);
                Ok((PublishDisposition::Created, Some(linked_identity)))
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                binding.verify()?;
                reconcile_existing(binding.target(), bytes).map(|disposition| (disposition, None))
            }
            Err(error) => Err(error),
        }
    })();

    if !linked {
        stage.set_len(0)?;
        stage.sync_all()?;
    }
    drop(stage);
    remove_if_same(&stage_path, &stage_identity)?;
    sync_directory(parent)?;
    let (disposition, linked_identity) = publish?;
    hook.reach(ReportPublicationBoundary::TerminalAcknowledgement)?;
    binding.verify()?;
    let (_, final_identity) =
        read_regular_bounded(binding.target(), MAX_REPORT_BYTES, "viewing report target")?;
    if linked_identity
        .as_ref()
        .is_some_and(|linked| !same_file_identity(linked, &final_identity))
    {
        return invalid("viewing report target changed before acknowledgement");
    }
    reconcile_existing(binding.target(), bytes)?;
    Ok(disposition)
}

fn create_stage(target: &Path) -> io::Result<(PathBuf, File)> {
    let file_name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "report target needs a name"))?;
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for _ in 0..MAX_STAGE_ATTEMPTS {
        let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
        let mut stage_name = file_name.to_os_string();
        stage_name.push(format!(".stage.{}.{sequence}", std::process::id()));
        let path = parent.join(stage_name);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a viewing report stage name",
    ))
}

fn reconcile_existing(path: &Path, expected: &[u8]) -> io::Result<PublishDisposition> {
    let (actual, _) = read_regular_bounded(path, MAX_REPORT_BYTES, "viewing report target")?;
    if actual == expected {
        Ok(PublishDisposition::Reconciled)
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "viewing report target exists with different bytes",
        ))
    }
}

fn read_regular_bounded(
    path: &Path,
    maximum: u64,
    role: &'static str,
) -> io::Result<(Vec<u8>, fs::Metadata)> {
    let initial = fs::symlink_metadata(path)?;
    if !initial.file_type().is_file() {
        return invalid(format!("{role} must be a regular non-symlink file"));
    }
    if initial.len() > maximum {
        return Err(out_of_memory());
    }
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    if !same_file_state(&initial, &opened) {
        return invalid(format!("{role} changed while opening"));
    }
    let capacity = usize::try_from(opened.len()).map_err(|_| out_of_memory())?;
    let bytes = read_exact_file_bytes(&mut file, capacity, role)?;
    let final_metadata = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !current.file_type().is_file()
        || !same_file_state(&opened, &final_metadata)
        || !same_file_state(&opened, &current)
    {
        return invalid(format!("{role} changed while reading"));
    }
    Ok((bytes, opened))
}

fn read_exact_file_bytes(file: &mut File, length: usize, role: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| out_of_memory())?;
    if bytes.capacity() > length {
        return Err(out_of_memory());
    }
    bytes.resize(length, 0);
    file.read_exact(&mut bytes)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return invalid(format!("{role} grew while reading"));
    }
    Ok(bytes)
}

fn out_of_memory() -> io::Error {
    io::Error::from(io::ErrorKind::OutOfMemory)
}

fn remove_if_same(path: &Path, expected: &fs::Metadata) -> io::Result<()> {
    let current = fs::symlink_metadata(path)?;
    if !current.file_type().is_file() || !same_file_identity(expected, &current) {
        return invalid("viewing report stage changed before retained cleanup");
    }
    // Retain the unique stage alias. A conditional unlink is unavailable, and
    // check-then-unlink or check-then-rename can both affect a raced caller path.
    // Once linked to the report target this alias retains no duplicate bytes.
    Ok(())
}

fn verify_directory(path: &Path, expected: &fs::Metadata) -> io::Result<()> {
    let current = fs::symlink_metadata(path)?;
    if current.file_type().is_dir() && same_file_identity(expected, &current) {
        Ok(())
    } else {
        invalid("viewing report parent directory changed identity")
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(unix)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(unix)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(windows)]
fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn same_file_state(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

fn validate_text(name: &'static str, text: &str, maximum: usize) -> io::Result<()> {
    if text.trim().is_empty() || text.len() > maximum {
        invalid(format!("{name} must contain 1 to {maximum} UTF-8 bytes"))
    } else {
        Ok(())
    }
}

fn validate_path(name: &'static str, path: &Path) -> io::Result<()> {
    let length = path.as_os_str().as_encoded_bytes().len();
    if length == 0 || length > MAX_PATH_BYTES {
        invalid(format!("{name} must contain 1 to {MAX_PATH_BYTES} bytes"))
    } else {
        Ok(())
    }
}

fn invalid<T>(message: impl Into<String>) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_protocol::ProtocolError;
    use render_wgpu::RendererError;

    const VALID_EMPTY_ENTRY_MANIFEST: &[u8] = br#"{"schema":"punctra.renderer-demo.field-corpus.v1","corpus_id":"c","machine":{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"},"entries":[{"id":"e","project_id":"p","firm_id":"f","source_path":"missing.laz","index_path":"index.pidx","inspect_permission":true,"measure_permission":true,"display":"neutral","projection":"perspective","trace":[]}]}"#;

    #[test]
    fn corpus_command_bounds_manifest_and_report_paths_before_use() {
        let exact_manifest = "m".repeat(MAX_PATH_BYTES);
        let exact_report = "r".repeat(MAX_PATH_BYTES);
        let command = CorpusCommand::parse([
            "--manifest".into(),
            exact_manifest.clone().into(),
            "--report".into(),
            exact_report.into(),
        ])
        .unwrap();
        assert_eq!(
            command.manifest().as_os_str().as_encoded_bytes().len(),
            MAX_PATH_BYTES
        );
        assert_eq!(
            command.report().as_os_str().as_encoded_bytes().len(),
            MAX_PATH_BYTES
        );

        let oversized_manifest = format!("{exact_manifest}x");
        let error = CorpusCommand::parse([
            "--manifest".into(),
            oversized_manifest.into(),
            "--report".into(),
            "report.json".into(),
        ])
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("corpus manifest path"));
        assert!(error.to_string().contains(&MAX_PATH_BYTES.to_string()));

        let oversized_report = "r".repeat(MAX_PATH_BYTES + 1);
        let error = CorpusCommand::parse([
            "--manifest".into(),
            "manifest.json".into(),
            "--report".into(),
            oversized_report.into(),
        ])
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("viewing report path"));
        assert!(error.to_string().contains(&MAX_PATH_BYTES.to_string()));

        let unrecognized = "x".repeat(MAX_PATH_BYTES + 1);
        let error = CorpusCommand::parse([unrecognized.into()]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "unrecognized corpus argument; expected --manifest or --report"
        );
    }

    #[test]
    fn manifest_rejects_permission_and_unknown_fields_before_any_source_access() {
        let directory = tempfile::tempdir().unwrap();
        let manifest_path = directory.path().join("manifest.json");
        fs::write(
            &manifest_path,
            br#"{"schema":"punctra.renderer-demo.field-corpus.v1","corpus_id":"c","machine":{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"},"entries":[{"id":"e","project_id":"p","firm_id":"f","source_path":"missing.laz","index_path":"missing.pidx","inspect_permission":false,"measure_permission":true,"display":"neutral","projection":"perspective","trace":[]}]}"#,
        )
        .unwrap();

        let error = CorpusManifest::load(&manifest_path).unwrap_err();
        assert!(error.to_string().contains("requires explicit"));

        let unknown_path = directory.path().join("unknown.json");
        fs::write(
            &unknown_path,
            br#"{"schema":"punctra.renderer-demo.field-corpus.v1","corpus_id":"c","machine":{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g","secret":"x"},"entries":[]}"#,
        )
        .unwrap();
        assert!(CorpusManifest::load(&unknown_path).is_err());
    }

    #[test]
    fn manifest_defaults_and_bounds_frames_per_pose() {
        let directory = tempfile::tempdir().unwrap();
        let default_path = directory.path().join("default.json");
        fs::write(
            &default_path,
            br#"{"schema":"punctra.renderer-demo.field-corpus.v1","corpus_id":"c","machine":{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"},"entries":[{"id":"e","project_id":"p","firm_id":"f","source_path":"source.las","index_path":"index.pidx","inspect_permission":true,"measure_permission":true,"display":"neutral","projection":"perspective","trace":[{"orbit_horizontal_pixels":0.0,"orbit_vertical_pixels":0.0,"pan_horizontal_pixels":0.0,"pan_vertical_pixels":0.0,"zoom_lines":0.0}]}]}"#,
        )
        .unwrap();
        let manifest = CorpusManifest::load(&default_path).unwrap();
        assert_eq!(
            manifest.entries[0].initial_frame_count(),
            DEFAULT_FRAMES_PER_POSE
        );
        assert_eq!(
            manifest.entries[0].trace()[0].frame_count,
            DEFAULT_FRAMES_PER_POSE
        );

        let invalid_path = directory.path().join("invalid.json");
        fs::write(
            &invalid_path,
            br#"{"schema":"punctra.renderer-demo.field-corpus.v1","corpus_id":"c","machine":{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"},"entries":[{"id":"e","project_id":"p","firm_id":"f","source_path":"source.las","index_path":"index.pidx","inspect_permission":true,"measure_permission":true,"display":"neutral","projection":"perspective","initial_frame_count":0,"trace":[]}]}"#,
        )
        .unwrap();
        assert!(
            CorpusManifest::load(&invalid_path)
                .unwrap_err()
                .to_string()
                .contains("frame_count")
        );
    }

    #[test]
    fn gpu_setup_failure_precedes_source_and_index_access() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("manifest.json");
        let report = directory.path().join("report.json");
        fs::write(&manifest, VALID_EMPTY_ENTRY_MANIFEST).unwrap();
        let command = CorpusCommand { manifest, report };

        let error = run_with_gpu(&command, || {
            Err(ViewFailure::corpus_gpu("injected adapter failure"))
        })
        .unwrap_err();
        let failure = error.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(failure.phase(), ViewPhase::GpuSetup);
        assert_eq!(
            failure.action(),
            crate::diagnostic::RecoveryAction::ConfigureCorpusGpu
        );
        assert!(!directory.path().join("index.pidx").exists());
        assert!(!command.report().exists());
    }

    #[test]
    fn renderer_setup_failure_precedes_source_and_index_access() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("manifest.json");
        let report = directory.path().join("report.json");
        fs::write(&manifest, VALID_EMPTY_ENTRY_MANIFEST).unwrap();
        let command = CorpusCommand { manifest, report };

        let error = prepare_corpus_runtime(
            &command,
            || Ok(()),
            |()| Err::<(), _>(ViewFailure::corpus_gpu("injected renderer setup failure")),
        )
        .unwrap_err();
        let failure = error.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(failure.phase(), ViewPhase::GpuSetup);
        assert_eq!(
            failure.action(),
            crate::diagnostic::RecoveryAction::ConfigureCorpusGpu
        );
        assert!(!directory.path().join("index.pidx").exists());
        assert!(!command.report().exists());
    }

    #[test]
    fn reusable_gpu_renderer_accepts_two_corpus_entry_generations() {
        let (device, _queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor {
            label: Some("renderer corpus generation test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        });
        let limits = RenderLimits::new(
            crate::synthetic::RESIDENT_BYTE_BUDGET,
            crate::synthetic::RESIDENT_POINT_BUDGET,
            crate::synthetic::RESIDENT_BATCH_BUDGET,
        );
        let mut renderer = WgpuRenderer::new(
            &device,
            RendererConfig::new(wgpu::TextureFormat::Rgba8Unorm, limits),
        )
        .unwrap();
        let first = corpus_view_generation(0);
        let second = corpus_view_generation(1);
        assert_eq!(first.view(), second.view());
        assert!(first.generation() < second.generation());

        for view_generation in [first, second] {
            renderer
                .apply(&RenderUpdate::Reset { view_generation })
                .unwrap();
        }
    }

    #[test]
    fn oversized_manifest_is_a_resource_failure() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("manifest.json");
        let file = File::create(&manifest).unwrap();
        file.set_len(MAX_MANIFEST_BYTES + 1).unwrap();
        let error = CorpusManifest::load(&manifest).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        let failure = manifest_loading_failure(&error);
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::RequestValidation);
    }

    #[test]
    fn manifest_deserialization_rejects_bounded_containers_during_parsing() {
        let directory = tempfile::tempdir().unwrap();
        let too_many_entries = directory.path().join("too-many-entries.json");
        let entry = r#"{"id":"e","project_id":"p","firm_id":"f","source_path":"source.las","index_path":"index.pidx","inspect_permission":true,"measure_permission":true,"display":"neutral","projection":"perspective","trace":[]}"#;
        let entries = std::iter::repeat_n(entry, MAX_ENTRIES + 1)
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            &too_many_entries,
            format!(
                r#"{{"schema":"{MANIFEST_SCHEMA}","corpus_id":"c","machine":{{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"}},"entries":[{entries}]}}"#
            ),
        )
        .unwrap();
        let error = CorpusManifest::load(&too_many_entries).unwrap_err();
        assert!(error.to_string().contains("exceeds 64 items"));

        let oversized_id = directory.path().join("oversized-id.json");
        fs::write(
            &oversized_id,
            format!(
                r#"{{"schema":"{MANIFEST_SCHEMA}","corpus_id":"{}","machine":{{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"}},"entries":[]}}"#,
                "x".repeat(MAX_ID_BYTES + 1)
            ),
        )
        .unwrap();
        let error = CorpusManifest::load(&oversized_id).unwrap_err();
        assert!(error.to_string().contains("1 to 128 UTF-8 bytes"));

        let spoofed_allocation = directory.path().join("spoofed-allocation.json");
        let spoofed = std::str::from_utf8(VALID_EMPTY_ENTRY_MANIFEST)
            .unwrap()
            .replace("\"neutral\"", "\"corpus manifest allocation failed\"");
        fs::write(&spoofed_allocation, spoofed).unwrap();
        let error = CorpusManifest::load(&spoofed_allocation).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            manifest_loading_failure(&error).code(),
            ViewFailureCode::InvalidRequest
        );

        let escaped_oversized = directory.path().join("escaped-oversized.json");
        let escaped_id = "\\u0061".repeat(MAX_ID_BYTES + 1);
        fs::write(
            &escaped_oversized,
            format!(
                r#"{{"schema":"{MANIFEST_SCHEMA}","corpus_id":"{escaped_id}","machine":{{"label":"m","operating_system":"o","filesystem":"f","gpu_expectation":"g"}},"entries":[]}}"#
            ),
        )
        .unwrap();
        let error = CorpusManifest::load(&escaped_oversized).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("1 to 128 UTF-8 bytes"));
        assert_eq!(
            manifest_loading_failure(&error).code(),
            ViewFailureCode::InvalidRequest
        );

        let oversized_decoded_string = format!(r#""{}""#, "\\u0061".repeat(MAX_PATH_BYTES + 1));
        let error = preflight_manifest_json_strings(oversized_decoded_string.as_bytes())
            .expect_err("decoded escaped text must retain the manifest string bound");
        assert!(error.to_string().contains("decoded JSON strings"));
    }

    #[test]
    fn manifest_accepts_bounded_standard_json_escapes() {
        let escaped = std::str::from_utf8(VALID_EMPTY_ENTRY_MANIFEST)
            .unwrap()
            .replace(r#""corpus_id":"c""#, r#""corpus_id":"\ud83d\ude80""#)
            .replace(r#""label":"m""#, r#""label":"m\u0022achine""#)
            .replace(
                r#""source_path":"missing.laz""#,
                r#""source_path":"C:\\field\\source.laz""#,
            );

        let manifest = parse_manifest_bytes(escaped.as_bytes(), &mut || true).unwrap();

        assert_eq!(manifest.corpus_id, "🚀");
        assert_eq!(manifest.machine.label, "m\"achine");
        assert_eq!(
            manifest.entries[0].source_path,
            PathBuf::from(r"C:\field\source.laz")
        );
    }

    #[test]
    fn manifest_materialization_failure_is_a_static_resource_error() {
        let mut outcome = None;
        let allocations = allocation_counter::measure(|| {
            outcome = Some(parse_manifest_bytes(
                VALID_EMPTY_ENTRY_MANIFEST,
                &mut || false,
            ));
        });
        let error = outcome
            .expect("the measured parse records its result")
            .expect_err("forced materialization failure must be typed as a resource limit");

        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(
            error.get_ref().is_none(),
            "manifest OOM must not allocate a Serde or custom error payload"
        );
        assert_eq!(allocations.count_total, 1);
        assert_eq!(allocations.count_current, 0);
        let failure = manifest_loading_failure(&error);
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::RequestValidation);
    }

    #[test]
    fn bounded_file_read_reservation_failure_is_out_of_memory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty");
        fs::write(&path, []).unwrap();
        let mut file = File::open(path).unwrap();

        let error = read_exact_file_bytes(&mut file, usize::MAX, "test input")
            .expect_err("an unaddressable allocation must fail before file reading");

        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(
            error.get_ref().is_none(),
            "reserve failure must not allocate a custom error payload"
        );
    }

    #[test]
    fn bounded_report_buffer_has_an_exact_capacity_and_static_reserve_failure() {
        let reserve_error = BoundedReportBuffer::new(u64::MAX)
            .err()
            .expect("an unaddressable report capacity must fail");
        assert_eq!(reserve_error.kind(), io::ErrorKind::OutOfMemory);
        assert!(reserve_error.get_ref().is_none());

        let mut exact = BoundedReportBuffer::new(4).unwrap();
        assert_eq!(exact.bytes.capacity(), 4);
        exact.write_all(b"four").unwrap();
        let over = exact
            .write_all(b"x")
            .expect_err("one byte over the report ceiling fails before growth");
        assert_eq!(over.kind(), io::ErrorKind::OutOfMemory);
        assert!(over.get_ref().is_none());
        assert_eq!(exact.into_inner(), b"four");
    }

    #[test]
    fn corpus_io_mappings_distinguish_resources_from_input_and_disk_failures() {
        let out_of_memory = io::Error::new(io::ErrorKind::OutOfMemory, "allocation failed");
        let invalid_data = io::Error::new(io::ErrorKind::InvalidData, "invalid manifest");
        let disk = io::Error::new(io::ErrorKind::PermissionDenied, "read-only directory");
        let conflict = io::Error::new(io::ErrorKind::AlreadyExists, "different report bytes");

        let manifest_resource = manifest_loading_failure(&out_of_memory);
        assert_eq!(manifest_resource.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(manifest_resource.phase(), ViewPhase::RequestValidation);
        assert_eq!(
            manifest_resource.action(),
            crate::diagnostic::RecoveryAction::RaiseNamedLimit
        );
        let manifest_invalid = manifest_loading_failure(&invalid_data);
        assert_eq!(manifest_invalid.code(), ViewFailureCode::InvalidRequest);
        assert_eq!(manifest_invalid.phase(), ViewPhase::RequestValidation);

        let report_resource =
            report_publication_failure(&ReportPublicationError::classify(out_of_memory));
        assert_eq!(report_resource.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(report_resource.phase(), ViewPhase::ReportPublication);
        let report_conflict =
            report_publication_failure(&ReportPublicationError::classify(conflict));
        assert_eq!(report_conflict.code(), ViewFailureCode::InvalidRequest);
        assert_eq!(report_conflict.phase(), ViewPhase::ReportPublication);
        assert_eq!(
            report_conflict.action(),
            crate::diagnostic::RecoveryAction::ChooseFreshCorpusTargets
        );
        let report_io = report_publication_failure(&ReportPublicationError::classify(disk));
        assert_eq!(report_io.code(), ViewFailureCode::Io);
        assert_eq!(report_io.phase(), ViewPhase::ReportPublication);
        assert_eq!(
            report_io.action(),
            crate::diagnostic::RecoveryAction::RestoreDisk
        );
    }

    #[cfg(unix)]
    #[test]
    fn report_invalid_targets_and_conflicts_share_fresh_target_recovery() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target_directory = directory.path().join("directory-target");
        fs::create_dir(&target_directory).unwrap();
        let symlink_target = directory.path().join("symlink-target");
        symlink(&target_directory, &symlink_target).unwrap();
        let missing_parent = directory.path().join("missing").join("report.json");
        let oversize = vec![0; usize::try_from(MAX_REPORT_BYTES).unwrap() + 1];
        let conflict_target = directory.path().join("conflict.json");
        fs::write(&conflict_target, b"caller bytes").unwrap();

        let errors = [
            publish_report(&target_directory, b"report\n").unwrap_err(),
            publish_report(&symlink_target, b"report\n").unwrap_err(),
            publish_report(&missing_parent, b"report\n").unwrap_err(),
            publish_report(directory.path().join("report.json").as_path(), &oversize).unwrap_err(),
            publish_report(&conflict_target, b"different\n").unwrap_err(),
        ];
        for error in errors {
            let failure = report_publication_failure(&error);
            assert!(matches!(
                failure.code(),
                ViewFailureCode::InvalidRequest | ViewFailureCode::ResourceLimit
            ));
            let failure = failure.for_completed_corpus();
            assert_eq!(
                failure.action(),
                crate::diagnostic::RecoveryAction::RetryCorpusWithFreshTargets
            );
        }
    }

    #[test]
    fn completed_corpus_failures_have_one_complete_retry_action() {
        for failure in [
            classify_renderer_failure(
                ViewPhase::GpuUpload,
                RendererError::Protocol(ProtocolError::ResidentLimitExceeded {
                    resource: render_protocol::ResidentResource::Points,
                    limit: 1,
                    attempted: 2,
                }),
            ),
            ViewFailure::source(
                Path::new("missing.las"),
                &point_source::SourceError::Cancelled,
            ),
            report_encoding_failure(&io::Error::new(io::ErrorKind::OutOfMemory, "report limit")),
        ] {
            let failure = failure.for_completed_corpus();
            assert_eq!(
                failure.action(),
                crate::diagnostic::RecoveryAction::RetryCorpusWithFreshTargets
            );
            assert_eq!(failure.to_string().matches("safe action:").count(), 1);
        }
    }

    #[test]
    fn first_visible_frame_is_latched_across_later_observations() {
        let entry = fixture_corpus_entry();
        let mut evidence = EntryEvidence::new(&entry);
        evidence.latch_first_visible_frame(std::time::Duration::from_nanos(7));
        evidence.latch_first_visible_frame(std::time::Duration::from_nanos(99));

        assert_eq!(
            evidence
                .finish(entry, None)
                .measurements
                .first_accepted_visible_batch_nanoseconds,
            Some(7)
        );
    }

    #[test]
    fn resource_failure_retains_completed_facts_and_typed_cause() {
        let entry = fixture_corpus_entry();
        let mut evidence = EntryEvidence::new(&entry);
        evidence
            .record_source(point_contracts::SourceId::new([1; 32]), 10, "las", 44)
            .unwrap();
        let failure = preserve_scene_failure(
            Box::new(ViewFailure::resource(
                ViewPhase::HostStaging,
                "staging byte ceiling reached",
            )),
            ViewPhase::Planning,
            "reconciliation",
        );

        let report = evidence.finish(entry, Some(&failure));
        let terminal = failure.reowned().for_completed_corpus();
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::HostStaging);
        assert_eq!(report.disposition, EntryDisposition::ResourceLimited);
        assert_eq!(
            report.source.source_identity.as_deref(),
            Some("0101010101010101010101010101010101010101010101010101010101010101")
        );
        assert_eq!(
            report.measurements.source_verification_nanoseconds,
            Some(44)
        );
        assert_eq!(report.measurements.index_prepare_nanoseconds, None);
        assert_eq!(report.measurements.index_warm_open_nanoseconds, None);
        let report_failure = report.failure.as_ref().unwrap();
        assert_eq!(report_failure.phase, "host-staging");
        assert_eq!(report_failure.safe_action, terminal.action().as_str());

        let summary = report_summary(
            std::slice::from_ref(&report),
            1,
            1,
            false,
            DEFAULT_SETTLEMENT_FRAME_CEILING,
            false,
        );
        let encoded = ViewingReport::new(
            "failed-corpus".into(),
            ReportMachine {
                declared_label: "machine".into(),
                declared_operating_system: "os".into(),
                declared_filesystem: "fs".into(),
                declared_gpu_expectation: "gpu".into(),
                observed_gpu_adapter: "adapter".into(),
                observed_gpu_backend: "backend",
                observed_depth_cue_status: "active",
            },
            summary,
            vec![report],
        )
        .encode()
        .unwrap();
        let encoded = String::from_utf8(encoded).unwrap();
        assert!(encoded.contains(&format!(
            "\"safe_action\":\"{}\"",
            terminal.action().as_str()
        )));
    }

    #[test]
    fn renderer_residency_limit_remains_a_resource_failure() {
        let failure = classify_renderer_failure(
            ViewPhase::GpuUpload,
            RendererError::Protocol(ProtocolError::ResidentLimitExceeded {
                resource: render_protocol::ResidentResource::Points,
                limit: 1,
                attempted: 2,
            }),
        );

        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::GpuUpload);
    }

    #[test]
    fn renderer_host_state_and_protocol_failures_remain_internal() {
        for error in [
            RendererError::NoActiveViewGeneration,
            RendererError::ForeignRecordedFrame,
            RendererError::Protocol(ProtocolError::EmptyPointBatch),
        ] {
            let failure = classify_renderer_failure(ViewPhase::Rendering, error);

            assert_eq!(failure.code(), ViewFailureCode::Internal);
            assert_eq!(failure.phase(), ViewPhase::Rendering);
        }
    }

    #[test]
    fn corpus_requires_a_fresh_cold_build_before_recording_a_warm_open() {
        assert!(require_cold_build(PrepareDisposition::Built).is_ok());
        for disposition in [PrepareDisposition::Opened, PrepareDisposition::Resumed] {
            let failure = require_cold_build(disposition).unwrap_err();
            assert_eq!(failure.code(), ViewFailureCode::InvalidRequest);
            assert_eq!(failure.phase(), ViewPhase::IndexPrepare);
            assert_eq!(
                failure.action(),
                crate::diagnostic::RecoveryAction::RebuildIndexExplicitly
            );
        }
    }

    #[test]
    fn canonical_report_omits_paths_and_reconciles_exact_bytes() {
        let report = ViewingReport::new(
            "corpus".into(),
            ReportMachine {
                declared_label: "machine".into(),
                declared_operating_system: "os".into(),
                declared_filesystem: "fs".into(),
                declared_gpu_expectation: "gpu".into(),
                observed_gpu_adapter: "adapter".into(),
                observed_gpu_backend: "backend",
                observed_depth_cue_status: "active",
            },
            ReportSummary {
                entry_count: 1,
                distinct_project_count: 1,
                distinct_firm_count: 1,
                passed_count: 1,
                failed_count: 0,
                resource_limited_count: 0,
                pre_v0_13_qualification: false,
                settlement_frame_ceiling: u64::from(DEFAULT_SETTLEMENT_FRAME_CEILING),
                quiet_observation_frames: 0,
                display_projection_matrix_complete: false,
                known_feature_located_count: 0,
                known_feature_issue_count: 0,
            },
            vec![fixture_entry()],
        );
        let first = report.encode().unwrap();
        let second = report.encode().unwrap();
        assert_eq!(first, second);
        let encoded = String::from_utf8(first.clone()).unwrap();
        assert!(!encoded.contains("/private/source.laz"));
        assert!(!encoded.contains("\"source_file_bytes\""));
        assert!(encoded.contains("\"peak_index_temporary_disk_bytes\":5"));
        assert!(!encoded.contains("verified_source_snapshot_bytes"));
        assert!(!encoded.contains("peak_total_temporary_disk_bytes"));
        assert!(encoded.contains("\"production_corpus_complete\":false"));

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("report.json");
        assert_eq!(
            publish_report(&target, &first).unwrap(),
            PublishDisposition::Created
        );
        assert_eq!(
            publish_report(&target, &first).unwrap(),
            PublishDisposition::Reconciled
        );
        assert_eq!(fs::read(&target).unwrap(), first);
        assert!(publish_report(&target, b"different\n").is_err());
    }

    #[test]
    fn report_encoder_accepts_exact_limit_and_rejects_before_over_limit_growth() {
        let report = ViewingReport::new(
            "corpus".into(),
            ReportMachine {
                declared_label: "machine".into(),
                declared_operating_system: "os".into(),
                declared_filesystem: "fs".into(),
                declared_gpu_expectation: "gpu".into(),
                observed_gpu_adapter: "adapter".into(),
                observed_gpu_backend: "backend",
                observed_depth_cue_status: "active",
            },
            ReportSummary {
                entry_count: 1,
                distinct_project_count: 1,
                distinct_firm_count: 1,
                passed_count: 1,
                failed_count: 0,
                resource_limited_count: 0,
                pre_v0_13_qualification: false,
                settlement_frame_ceiling: u64::from(DEFAULT_SETTLEMENT_FRAME_CEILING),
                quiet_observation_frames: 0,
                display_projection_matrix_complete: false,
                known_feature_located_count: 0,
                known_feature_issue_count: 0,
            },
            vec![fixture_entry()],
        );
        let encoded = report.encode().unwrap();

        assert_eq!(
            encode_report_bounded(&report, encoded.len() as u64).unwrap(),
            encoded
        );
        let error = encode_report_bounded(&report, encoded.len() as u64 - 1)
            .expect_err("one byte below the exact report shape fails closed");
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
        assert!(error.get_ref().is_none());
        let failure = report_encoding_failure(&error);
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::ReportPublication);
    }

    #[test]
    fn pre_v0_13_manifest_requires_the_complete_mode_projection_and_feature_matrix() {
        let mut manifest = qualification_manifest();
        assert!(manifest.validate().is_ok());
        assert!(manifest.display_projection_matrix_complete());
        assert!(manifest.known_feature_matrix_complete());

        manifest.entries.pop();
        let error = manifest.validate().unwrap_err();
        assert!(error.to_string().contains("all five display modes"));
    }

    #[cfg(unix)]
    #[test]
    fn report_publication_rejects_ancestor_retarget_before_and_after_link() {
        use std::os::unix::fs::symlink;

        for boundary in [
            ReportPublicationBoundary::BeforeLink,
            ReportPublicationBoundary::AfterLink,
            ReportPublicationBoundary::TerminalAcknowledgement,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let outside = directory.path().join("outside");
            let redirected = directory.path().join("redirected");
            let alias = directory.path().join("report-parent");
            fs::create_dir(&outside).unwrap();
            fs::create_dir(&redirected).unwrap();
            symlink(&outside, &alias).unwrap();
            let requested_target = alias.join("sensitive-report.json");
            let binding = BoundReportTarget::bind(&requested_target).unwrap();

            let error = publish_bound_report(
                &binding,
                b"sensitive viewing report\n",
                &RetargetAncestorHook {
                    boundary,
                    alias: &alias,
                    replacement: &redirected,
                },
            )
            .expect_err("retargeting the requested ancestor invalidates its binding");

            assert!(
                error.to_string().contains("binding") || error.kind() == io::ErrorKind::NotFound,
                "unexpected retarget error: {error}"
            );
            assert!(
                !redirected.join("sensitive-report.json").exists(),
                "the report must never follow the retargeted ancestor"
            );
            if boundary == ReportPublicationBoundary::BeforeLink {
                assert!(!outside.join("sensitive-report.json").exists());
            } else {
                assert_eq!(
                    fs::read(outside.join("sensitive-report.json")).unwrap(),
                    b"sensitive viewing report\n"
                );
            }
            let stages = fs::read_dir(&outside)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".stage."))
                .collect::<Vec<_>>();
            assert_eq!(stages.len(), 1, "one unique stage alias is retained");
            let stage = &stages[0];
            assert!(stage.file_type().unwrap().is_file());
            let target = outside.join("sensitive-report.json");
            if target.exists() {
                assert!(same_file_identity(
                    &stage.metadata().unwrap(),
                    &fs::metadata(target).unwrap()
                ));
            } else {
                assert_eq!(stage.metadata().unwrap().len(), 0);
            }
        }
    }

    #[cfg(unix)]
    struct RetargetAncestorHook<'a> {
        boundary: ReportPublicationBoundary,
        alias: &'a Path,
        replacement: &'a Path,
    }

    #[cfg(unix)]
    impl ReportPublicationHook for RetargetAncestorHook<'_> {
        fn reach(&self, boundary: ReportPublicationBoundary) -> io::Result<()> {
            if self.boundary == boundary {
                fs::remove_file(self.alias)?;
                std::os::unix::fs::symlink(self.replacement, self.alias)?;
            }
            Ok(())
        }
    }

    fn fixture_entry() -> EntryReport {
        EntryReport {
            id: "entry".into(),
            source: SourceEvidence {
                source_identity: Some("00".repeat(32)),
                source_point_count: Some(10),
                source_format: Some("laz".into()),
            },
            index: IndexEvidence {
                index_recipe_version: 2,
                index_disk_version: 2,
                index_disposition: Some("opened"),
            },
            display: DisplayChoice::Elevation,
            projection: ProjectionChoice::Orthographic,
            known_feature_outcomes: Vec::new(),
            declared_initial_frame_count: 2,
            declared_trace: vec![TraceStep {
                orbit_horizontal_pixels: 1.0,
                orbit_vertical_pixels: 2.0,
                pan_horizontal_pixels: 3.0,
                pan_vertical_pixels: 4.0,
                zoom_lines: 5.0,
                frame_count: 2,
            }],
            measurements: MeasurementEvidence {
                source_verification_nanoseconds: Some(1),
                index_prepare_nanoseconds: Some(2),
                index_warm_open_nanoseconds: Some(3),
                first_accepted_visible_batch_nanoseconds: Some(4),
                index_artifact_bytes: Some(4),
                peak_index_temporary_disk_bytes: Some(5),
            },
            limits: EffectiveLimits::current(),
            residency: ResidencyEvidence {
                peak_queued_batches: 1,
                peak_queued_host_bytes: 128,
                peak_staged_points: 10,
                peak_staged_bytes: 240,
                resident_batches: 1,
                resident_points: 10,
                sampled_resident_points: 10,
                complete_resident_points: 0,
                retired_batches: 0,
                cancelled_requests: 0,
                rejected_batches: 0,
                cumulative_uploaded_batches: 1,
                cumulative_uploaded_points: 10,
                cumulative_uploaded_bytes: 240,
                peak_resident_batches: 1,
                peak_resident_points: 10,
                peak_resident_bytes: 240,
            },
            trace: vec![TraceReport {
                step: 0,
                input: TraceStep::stationary(2),
                requested_frame_count: 2,
                completed_frame_count: 2,
                peak_demanded_nodes: 1,
                peak_issued_nodes_per_frame: 1,
                issued_nodes: 1,
                retired_nodes: 0,
                presentation_updates: 0,
                accepted_batches: 1,
                resident_batches: 1,
                resident_points: 10,
                drawn_points: 10,
                peak_resident_batches: 1,
                peak_resident_points: 10,
                peak_resident_bytes: 240,
                peak_transient_texture_bytes: 8 * 640 * 480,
                submitted_frame_nanoseconds: 6,
                settlement_frame: None,
                settlement_nanoseconds: None,
                quiet_observation_frames: 0,
                quiet_window_complete: false,
            }],
            disposition: EntryDisposition::Passed,
            failure: None,
        }
    }

    fn qualification_manifest() -> CorpusManifest {
        let mut entries = Vec::new();
        for (display_index, display) in DisplayChoice::ALL.into_iter().enumerate() {
            for (projection_index, projection) in ProjectionChoice::ALL.into_iter().enumerate() {
                let index = display_index * ProjectionChoice::ALL.len() + projection_index;
                entries.push(CorpusEntry {
                    id: format!("entry-{index}"),
                    project_id: format!("project-{}", index % 5),
                    firm_id: format!("firm-{}", index % 3),
                    source_path: PathBuf::from(format!("source-{index}.laz")),
                    index_path: PathBuf::from(format!("index-{index}.pidx")),
                    inspect_permission: true,
                    measure_permission: true,
                    display,
                    projection,
                    known_feature_outcomes: if index == 0 {
                        KnownFeatureKind::ALL
                            .into_iter()
                            .map(|kind| KnownFeatureOutcome {
                                kind,
                                result: KnownFeatureResult::Located,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    initial_frame_count: DEFAULT_FRAMES_PER_POSE,
                    trace: Vec::new(),
                });
            }
        }
        CorpusManifest {
            schema: MANIFEST_SCHEMA.to_owned(),
            corpus_id: "qualified-corpus".to_owned(),
            machine: MachineDeclaration {
                label: "machine".to_owned(),
                operating_system: "os".to_owned(),
                filesystem: "filesystem".to_owned(),
                gpu_expectation: "gpu".to_owned(),
            },
            entries,
            pre_v0_13_qualification: true,
            settlement_frame_ceiling: MAX_SETTLEMENT_FRAME_CEILING,
        }
    }

    fn fixture_corpus_entry() -> CorpusEntry {
        CorpusEntry {
            id: "entry".into(),
            project_id: "private-project".into(),
            firm_id: "private-firm".into(),
            source_path: PathBuf::from("/private/source.laz"),
            index_path: PathBuf::from("/private/index.pidx"),
            inspect_permission: true,
            measure_permission: true,
            display: DisplayChoice::Neutral,
            projection: ProjectionChoice::Perspective,
            known_feature_outcomes: Vec::new(),
            initial_frame_count: DEFAULT_FRAMES_PER_POSE,
            trace: Vec::new(),
        }
    }
}
