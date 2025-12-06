Name:           waytray
Version:        1.0.0
Release:        1%{?dist}
Summary:        Accessible system tray for Wayland

License:        MIT
URL:            https://github.com/destructatron/waytray
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gtk4-devel
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
* Sat Dec 06 2025 Harley Richardson <hrichardson2004@hotmail.com> - 1.0.0-1
- Initial package
