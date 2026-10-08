use libloading::Library;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowSystem {
    Wayland,
    X11,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayEnv {
    pub wayland: bool,
    pub x11: bool,
}

impl DisplayEnv {
    pub fn any(self) -> bool {
        self.wayland || self.x11
    }

    pub fn window_system(self, forced: Option<WindowSystem>) -> WindowSystem {
        match forced {
            Some(system) => system,
            None if self.wayland => WindowSystem::Wayland,
            None => WindowSystem::X11,
        }
    }
}

pub fn display_env() -> DisplayEnv {
    DisplayEnv {
        wayland: super::env_set("WAYLAND_DISPLAY") || super::env_set("WAYLAND_SOCKET"),
        x11: super::env_set("DISPLAY"),
    }
}

pub fn running_as_root() -> bool {
    // SAFETY: geteuid yan etkisizdir ve her zaman başarılı olur.
    unsafe { libc::geteuid() == 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Distro {
    Debian,
    Fedora,
    Arch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeLib {
    soname: &'static str,
    debian: &'static str,
    fedora: &'static str,
    arch: &'static str,
}

impl RuntimeLib {
    const fn new(
        soname: &'static str,
        debian: &'static str,
        fedora: &'static str,
        arch: &'static str,
    ) -> Self {
        Self {
            soname,
            debian,
            fedora,
            arch,
        }
    }

    fn package(self, distro: Distro) -> &'static str {
        match distro {
            Distro::Debian => self.debian,
            Distro::Fedora => self.fedora,
            Distro::Arch => self.arch,
        }
    }
}

const XKBCOMMON: RuntimeLib = RuntimeLib::new(
    "libxkbcommon.so.0",
    "libxkbcommon0",
    "libxkbcommon",
    "libxkbcommon",
);
const WAYLAND_CLIENT: RuntimeLib = RuntimeLib::new(
    "libwayland-client.so.0",
    "libwayland-client0",
    "libwayland-client",
    "wayland",
);
const WAYLAND_EGL: RuntimeLib = RuntimeLib::new(
    "libwayland-egl.so.1",
    "libwayland-egl1",
    "libwayland-egl",
    "wayland",
);
const EGL: RuntimeLib = RuntimeLib::new("libEGL.so.1", "libegl1", "libglvnd-egl", "libglvnd");
const GLX: RuntimeLib = RuntimeLib::new("libGL.so.1", "libgl1", "libglvnd-glx", "libglvnd");
const X11: RuntimeLib = RuntimeLib::new("libX11.so.6", "libx11-6", "libX11", "libx11");
const X11_XCB: RuntimeLib =
    RuntimeLib::new("libX11-xcb.so.1", "libx11-xcb1", "libX11-xcb", "libx11");
const XCB: RuntimeLib = RuntimeLib::new("libxcb.so.1", "libxcb1", "libxcb", "libxcb");
const XCURSOR: RuntimeLib =
    RuntimeLib::new("libXcursor.so.1", "libxcursor1", "libXcursor", "libxcursor");
const XINPUT: RuntimeLib = RuntimeLib::new("libXi.so.6", "libxi6", "libXi", "libxi");
const XKBCOMMON_X11: RuntimeLib = RuntimeLib::new(
    "libxkbcommon-x11.so.0",
    "libxkbcommon-x11-0",
    "libxkbcommon-x11",
    "libxkbcommon-x11",
);

const WAYLAND_NEEDS: &[&[RuntimeLib]] = &[&[XKBCOMMON], &[WAYLAND_CLIENT], &[WAYLAND_EGL], &[EGL]];
const X11_NEEDS: &[&[RuntimeLib]] = &[
    &[XKBCOMMON],
    &[XKBCOMMON_X11],
    &[X11],
    &[X11_XCB],
    &[XCB],
    &[XCURSOR],
    &[XINPUT],
    &[GLX, EGL],
];

fn requirements(system: WindowSystem) -> &'static [&'static [RuntimeLib]] {
    match system {
        WindowSystem::Wayland => WAYLAND_NEEDS,
        WindowSystem::X11 => X11_NEEDS,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissingLibs {
    pub sonames: Vec<&'static str>,
    pub debian: String,
    pub fedora: String,
    pub arch: String,
}

impl MissingLibs {
    pub fn is_empty(&self) -> bool {
        self.sonames.is_empty()
    }
}

pub fn missing_runtime_libs(system: WindowSystem) -> MissingLibs {
    describe(&missing_with(system, library_loads))
}

fn describe(libs: &[RuntimeLib]) -> MissingLibs {
    MissingLibs {
        sonames: libs.iter().map(|lib| lib.soname).collect(),
        debian: package_list(libs, Distro::Debian),
        fedora: package_list(libs, Distro::Fedora),
        arch: package_list(libs, Distro::Arch),
    }
}

fn missing_with(system: WindowSystem, mut loads: impl FnMut(&str) -> bool) -> Vec<RuntimeLib> {
    requirements(system)
        .iter()
        .filter(|choices| !choices.iter().any(|lib| loads(lib.soname)))
        .filter_map(|choices| choices.first().copied())
        .collect()
}

fn library_loads(soname: &str) -> bool {
    // SAFETY: pencere kitaplığının birazdan yükleyeceği sistem kütüphanesi
    // açılır; yalnız kendi başlatıcıları çalışır ve hiçbir sembol çağrılmaz.
    match unsafe { Library::new(soname) } {
        Ok(library) => {
            std::mem::forget(library);
            true
        }
        Err(_) => false,
    }
}

fn package_list(libs: &[RuntimeLib], distro: Distro) -> String {
    let mut packages: Vec<&str> = Vec::new();
    for lib in libs {
        let package = lib.package(distro);
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    packages.join(" ")
}
