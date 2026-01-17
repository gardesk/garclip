Name:           garclip
Version:        0.1.0
Release:        1%{?dist}
Summary:        Clipboard manager for the gar desktop suite

License:        MIT
URL:            https://github.com/gardesk/garclip
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  rust >= 1.75
BuildRequires:  cargo
BuildRequires:  libxcb-devel

# Disable debug package
%global debug_package %{nil}

%description
Garclip is a clipboard manager for X11 built in Rust. Features clipboard
history, a picker UI, and CLI control. Part of the gardesk desktop environment
suite.

%prep
%autosetup

%build
export CARGO_TARGET_DIR=target
cargo build --release --workspace

%install
install -Dm755 target/release/garclip %{buildroot}%{_bindir}/garclip
install -Dm755 target/release/garclipctl %{buildroot}%{_bindir}/garclipctl
install -Dm755 target/release/garclip-picker %{buildroot}%{_bindir}/garclip-picker

%files
%{_bindir}/garclip
%{_bindir}/garclipctl
%{_bindir}/garclip-picker

%changelog
* Fri Jan 17 2025 mfw <espadonne@outlook.com> - 0.1.0-1
- Initial RPM release of garclip
- Clipboard manager with history and picker UI
- Part of gardesk desktop suite
