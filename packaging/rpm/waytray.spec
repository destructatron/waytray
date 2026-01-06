Name:           waytray
Version:        1.0.5
Release:        1%{?dist}
Summary:        Accessible system tray for Wayland

License:        MIT
URL:            https://github.com/destructatron/waytray
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gtk4-devel
BuildRequires:  gtk4-layer-shell-devel
BuildRequires:  gstreamer1-devel
BuildRequires:  pkgconfig

Requires:       pulseaudio-utils

%description
WayTray is a compositor-agnostic Linux system tray with a daemon + client
architecture designed for accessibility. Features include system tray support
via StatusNotifierItem protocol, battery monitoring, audio volume control,
network status, weather, and more.

%prep
%autosetup

%build
cargo build --release

%install
install -Dm755 target/release/waytray %{buildroot}%{_bindir}/waytray
install -Dm755 target/release/waytray-daemon %{buildroot}%{_bindir}/waytray-daemon

%files
%license LICENSE
%doc README.md
%{_bindir}/waytray
%{_bindir}/waytray-daemon

%changelog
* Tue Jan 06 2026 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.5-1
- Add privacy module to show microphone usage

* Mon Dec 15 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.4-1
- Add GPU module for monitoring GPU usage and temperature

* Sun Dec 14 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.3-1
- Add GTK Layer Shell support for Wayland overlay windows

* Fri Dec 12 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.2-1
- Fix window closing unexpectedly when using arrow keys after context menu

* Sun Dec 07 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.1-1
- Close client window when focus leaves (tray-like behavior)

* Sat Dec 06 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.0-1
- Initial package
