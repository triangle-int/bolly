use std::{fs, io, path::Path};

pub fn count_directories(path: &Path) -> io::Result<usize> {
    Ok(fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count())
}
