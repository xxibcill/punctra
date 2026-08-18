//! Shared headless GPU setup for workspace acceptance tests.

use std::{
    env,
    sync::OnceLock,
    time::{Duration, Instant},
};

static GPU: OnceLock<Option<GpuContext>> = OnceLock::new();

pub(super) struct GpuContext {
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
}

impl GpuContext {
    pub(super) fn wait(&self) {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("headless device polling should succeed");
    }

    #[allow(
        dead_code,
        reason = "the shared path module is compiled by GPU tests that only need wait"
    )]
    pub(super) fn wait_for_submission<T>(
        &self,
        submission: &wgpu::SubmissionIndex,
        timeout: Duration,
        label: &str,
        mut poll: impl FnMut() -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(result) = poll() {
                return result;
            }
            let now = Instant::now();
            assert!(
                now < deadline,
                "{label} did not complete within {timeout:?}"
            );
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: Some(deadline.saturating_duration_since(now)),
                })
                .unwrap_or_else(|error| panic!("{label} device polling failed: {error}"));
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

pub(super) fn with_gpu(test: impl FnOnce(&GpuContext)) {
    if let Some(gpu) = GPU.get_or_init(initialize_gpu).as_ref() {
        test(gpu);
    }
}

fn initialize_gpu() -> Option<GpuContext> {
    pollster::block_on(request_gpu())
}

async fn request_gpu() -> Option<GpuContext> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await;
    let adapter = match adapter {
        Ok(adapter) => adapter,
        Err(error) if gpu_is_required() => {
            panic!("PUNCTRA_REQUIRE_GPU=1 but no headless adapter is available: {error}");
        }
        Err(error) => {
            eprintln!(
                "skipping GPU acceptance tests because no headless adapter is available: {error}"
            );
            return None;
        }
    };
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("punctra GPU acceptance test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .expect("a discovered headless adapter should provide a baseline device");
    Some(GpuContext { device, queue })
}

fn gpu_is_required() -> bool {
    env::var("PUNCTRA_REQUIRE_GPU").is_ok_and(|value| value == "1")
}
