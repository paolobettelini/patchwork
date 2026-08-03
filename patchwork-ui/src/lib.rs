pub mod catalog;
pub mod icons;
pub mod profile;
pub mod project;

pub use catalog::{BrowsePage, UploadPage};
pub use icons::{
    ArrowRightToBracketIcon, GithubIcon, HomeIcon, RefreshCwIcon, SearchIcon, UploadIcon, UserIcon,
};
pub use profile::{ProfilePage, PublishedProject};
pub use project::RegistryProjectPage;

pub const THEMES: [(&str, &str); 8] = [
    ("dark", "Dark"),
    ("dim-white", "Dim White"),
    ("aurora", "Aurora"),
    ("volcanic", "Volcanic"),
    ("nebula", "Nebula"),
    ("moss", "Moss"),
    ("bubblegum", "Bubblegum"),
    ("terminal", "Terminal"),
];
