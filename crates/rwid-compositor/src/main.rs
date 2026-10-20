static POSSIBLE_BACKENDS: &[&str] = &[
    #[cfg(feature = "winit")]
    "winit : run nested inside another X11/Wayland session (development).",
    #[cfg(feature = "udev")]
    "udev  : run on a TTY with DRM/KMS + libinput (the RWID/OS session).",
    #[cfg(feature = "x11")]
    "x11   : run as an X11 client.",
];

#[cfg(feature = "profile-with-tracy-mem")]
#[global_allocator]
static GLOBAL: profiling::tracy_client::ProfiledAllocator<std::alloc::System> =
    profiling::tracy_client::ProfiledAllocator::new(std::alloc::System, 10);

fn main() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt().compact().init();
    }

    #[cfg(feature = "profile-with-tracy")]
    profiling::tracy_client::Client::start();

    profiling::register_thread!("Main Thread");

    // `--backend <name>`; the bare anvil-style flags stay accepted.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let backend = match args.first().map(String::as_str) {
        Some("--backend") => args.get(1).cloned(),
        Some(flag) => Some(
            flag.trim_start_matches("--")
                .trim_start_matches("tty-")
                .to_owned(),
        ),
        None => None,
    };
    match backend.as_deref() {
        #[cfg(feature = "winit")]
        Some("winit") => {
            tracing::info!("Starting rwid-compositor with the winit backend");
            rwid_compositor::winit::run_winit();
        }
        #[cfg(feature = "udev")]
        Some("udev") => {
            tracing::info!("Starting rwid-compositor on a TTY (udev/DRM)");
            rwid_compositor::udev::run_udev();
        }
        #[cfg(feature = "x11")]
        Some("x11") => {
            tracing::info!("Starting rwid-compositor with the x11 backend");
            rwid_compositor::x11::run_x11();
        }
        other => {
            if let Some(other) = other {
                tracing::error!("Unknown backend: {other}");
            }
            tracing::error!("Usage: rwid-compositor --backend <name>");
            for b in POSSIBLE_BACKENDS {
                tracing::error!("  {b}");
            }
            std::process::exit(2);
        }
    }
}
