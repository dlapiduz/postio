use std::path::{Path, PathBuf};

pub const SYSTEM: &[&str] = &[
    "/etc/pki/ca-trust/extracted/pem/email-ca-bundle.pem",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/ssl/certs",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchors {
    pub from: Option<PathBuf>,
    pub certificates: Vec<Vec<u8>>,
}

pub fn read(candidates: &[PathBuf]) -> Anchors {
    for path in candidates {
        let certificates = if path.is_dir() {
            in_directory(path)
        } else {
            match std::fs::read(path) {
                Ok(bytes) => postio_signature::smime::anchors_in(&bytes),
                Err(_) => continue,
            }
        };
        if !certificates.is_empty() {
            return Anchors {
                from: Some(path.clone()),
                certificates,
            };
        }
    }
    Anchors::default()
}

pub fn system() -> Anchors {
    let anchors = read(&SYSTEM.iter().map(PathBuf::from).collect::<Vec<_>>());
    tracing::info!(
        certificates = anchors.certificates.len(),
        from = ?anchors.from,
        "S/MIME trust anchors read"
    );
    anchors
}

fn in_directory(dir: &Path) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "pem" || extension == "crt")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .flat_map(|bytes| postio_signature::smime::anchors_in(&bytes))
        .collect()
}
