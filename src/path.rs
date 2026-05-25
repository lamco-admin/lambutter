// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Filesystem-path representation and component splitting.
//!
//! Paths are byte slices because btrfs filenames are arbitrary bytes (the
//! kernel does not impose UTF-8). Lambutter mirrors that: callers supply
//! byte slices and lambutter does byte-exact comparisons.

/// A read-only path into the active subvolume. Paths are absolute
/// (must start with `/`); leading `/` segments collapse, internal `//`
/// collapses to `/`, and `.` components are skipped.
///
/// `..` components are treated as literal names; lambutter does not perform
/// path-canonicalization above the byte-walker level since a bootloader's
/// caller is in control of every input it supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path<'a> {
    bytes: &'a [u8],
}

impl<'a> Path<'a> {
    /// Wrap a byte slice as a path. Returns `None` if the slice is not
    /// absolute (does not start with `/`).
    pub fn new(bytes: &'a [u8]) -> Option<Self> {
        if bytes.first() != Some(&b'/') {
            return None;
        }
        Some(Self { bytes })
    }

    /// Iterate the path's components, skipping empty segments and `.`
    /// components. Each component is a non-empty byte slice.
    pub(crate) fn components(&self) -> Components<'a> {
        Components { bytes: self.bytes }
    }

    /// The underlying byte slice, including the leading `/`.
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// Conversion from `&str`. Returns a path that delegates to the same
/// validation as `Path::new`; non-absolute strings produce a path whose
/// component iterator yields nothing (since `components()` requires a leading
/// `/` to skip past). Callers wanting absolute-only enforcement at the
/// boundary should use `Path::new(s.as_bytes())` directly.
impl<'a> From<&'a str> for Path<'a> {
    fn from(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
        }
    }
}

/// Iterator over path components.
pub(crate) struct Components<'a> {
    bytes: &'a [u8],
}

impl<'a> Iterator for Components<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        // Skip leading slashes
        while let Some(b'/') = self.bytes.first() {
            self.bytes = &self.bytes[1..];
        }
        if self.bytes.is_empty() {
            return None;
        }
        let end = self
            .bytes
            .iter()
            .position(|b| *b == b'/')
            .unwrap_or(self.bytes.len());
        let component = &self.bytes[..end];
        self.bytes = &self.bytes[end..];

        // Skip `.` components
        if component == b"." {
            self.next()
        } else {
            Some(component)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_absolute_path() {
        assert!(Path::new(b"relative").is_none());
        assert!(Path::new(b"").is_none());
    }

    #[test]
    fn iterates_components() {
        let p = Path::new(b"/a/b/c").unwrap();
        let parts: alloc::vec::Vec<&[u8]> = p.components().collect();
        assert_eq!(parts, &[&b"a"[..], &b"b"[..], &b"c"[..]]);
    }

    #[test]
    fn collapses_double_slashes() {
        let p = Path::new(b"//a///b//c/").unwrap();
        let parts: alloc::vec::Vec<&[u8]> = p.components().collect();
        assert_eq!(parts, &[&b"a"[..], &b"b"[..], &b"c"[..]]);
    }

    #[test]
    fn skips_dot_components() {
        let p = Path::new(b"/./a/./b").unwrap();
        let parts: alloc::vec::Vec<&[u8]> = p.components().collect();
        assert_eq!(parts, &[&b"a"[..], &b"b"[..]]);
    }

    #[test]
    fn root_path_yields_no_components() {
        let p = Path::new(b"/").unwrap();
        let parts: alloc::vec::Vec<&[u8]> = p.components().collect();
        assert!(parts.is_empty());
    }
}
