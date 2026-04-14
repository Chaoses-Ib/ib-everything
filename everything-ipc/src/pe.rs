/*!
PE file utilities.
*/

use std::{io, path::Path};

use pelite::{PeFile, pe32::image::VS_FIXEDFILEINFO};

use crate::Version;

/// Error type for PE version extraction.
#[derive(Debug, thiserror::Error)]
pub enum PeVersionError {
    #[error("failed to read: {0}")]
    Io(#[from] io::Error),

    #[error("failed to parse PE: {0}")]
    Parse(#[from] pelite::Error),

    #[error("failed to find version: {0}")]
    Resources(#[from] pelite::resources::FindError),

    #[error("version not found")]
    NotFound,
}

impl Version {
    /// Get the version information from the current executable.
    ///
    /// Mainly for plugin usage. Can be used before plugin init.
    ///
    /// # Returns
    ///
    /// `Ok(version)` if version info is found, or `Err(e)` if no version info or reading/parsing fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// let version = everything_ipc::Version::from_current_exe()?;
    /// println!("Version: {:?}", version);
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn from_current_exe() -> Result<Self, PeVersionError> {
        let exe = std::env::current_exe()?;
        Self::from_exe(&exe)
    }

    /// Get the version information from a PE executable.
    ///
    /// # Arguments
    ///
    /// - `exe_path` - Path to the PE executable file.
    ///
    /// # Returns
    ///
    /// `Ok(version)` if version info is found, or `Err(e)` if no version info or reading/parsing fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// let version = everything_ipc::Version::from_exe(std::path::Path::new(r"C:\Path\To\Everything.exe"))?;
    /// println!("Version: {:?}", version);
    /// # Result::<_, Box<dyn std::error::Error>>::Ok(())
    /// ```
    pub fn from_exe(exe_path: &Path) -> Result<Self, PeVersionError> {
        let image = std::fs::read(exe_path)?;
        let pe = PeFile::from_bytes(&image)?;
        let resources = pe.resources()?;
        let version_info = resources.version_info()?;

        let fixed = version_info.fixed();
        let (major, minor, patch, build) = decode_version(fixed);

        // Return error if all version components are zero
        if major == 0 && minor == 0 && patch == 0 && build == 0 {
            Err(PeVersionError::NotFound)
        } else {
            Ok(Self {
                major,
                minor,
                revision: patch,
                build,
            })
        }
    }
}

/// Decode version from VS_FIXEDFILEINFO.
fn decode_version(fixed: Option<&VS_FIXEDFILEINFO>) -> (u32, u32, u32, u32) {
    match fixed {
        Some(info) => {
            // Prefer product version, fall back to file version if product is all zeros
            let version = if info.dwProductVersion.Major == 0
                && info.dwProductVersion.Minor == 0
                && info.dwProductVersion.Patch == 0
                && info.dwProductVersion.Build == 0
            {
                info.dwFileVersion
            } else {
                info.dwProductVersion
            };

            // The order is different from its physical layout
            (
                version.Major as u32,
                version.Minor as u32,
                version.Patch as u32,
                version.Build as u32,
            )
        }
        None => (0, 0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ignore]
    #[test]
    fn get_exe_version() {
        let version = Version::from_exe(Path::new(
            r"../tests/everything/v1.5.0.1403_x64/Everything.exe",
        ))
        .expect("read exe");
        assert_eq!(version.tuple(), (1, 5, 0, 1403));
    }
}
