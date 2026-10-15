//! Utilities for working with Protobuf paths.

use std::iter;

/// Maps a fully-qualified Protobuf path to a value using path matchers.
#[derive(Debug, Default)]
pub(crate) struct PathMap<T> {
    matchers: Vec<(String, T)>,
}

impl<T> PathMap<T> {
    /// Inserts a new matcher and associated value to the path map.
    pub(crate) fn insert(&mut self, matcher: String, value: T) {
        self.matchers.push((matcher, value));
    }

    /// Returns the best match for `fq_path.field`: the exact path, then suffixes,
    /// prefixes, and finally the global path. The first matching insertion wins.
    pub(crate) fn get_first_field<'a>(
        &'a self,
        fq_path: &'_ str,
        field: &'_ str,
    ) -> Option<&'a T> {
        let full_path = format!("{fq_path}.{field}");
        sub_path_iter(&full_path).find_map(|path| {
            self.matchers
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, v)| v)
        })
    }
}

/// Given a fully-qualified path, returns a sequence of paths:
/// - the path itself
/// - the sequence of suffix paths
/// - the sequence of prefix paths
/// - the global path
///
/// Example: `sub_path_iter(".a.b.c") -> [".a.b.c", "a.b.c", "b.c", "c", ".a.b", ".a", "."]`
fn sub_path_iter(full_path: &str) -> impl Iterator<Item = &str> {
    // First, try matching the path.
    iter::once(full_path)
        // Then, try matching path suffixes.
        .chain(suffixes(full_path))
        // Then, try matching path prefixes.
        .chain(prefixes(full_path))
        // Then, match the global path.
        .chain(iter::once("."))
}

/// Given a fully-qualified path, returns a sequence of fully-qualified paths which match a prefix
/// of the input path, in decreasing path-length order.
///
/// Example: `prefixes(".a.b.c.d") -> [".a.b.c", ".a.b", ".a"]`
fn prefixes(fq_path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(fq_path), |path| {
        #[allow(unknown_lints, clippy::manual_split_once)]
        path.rsplitn(2, '.').nth(1).filter(|path| !path.is_empty())
    })
    .skip(1)
}

/// Given a fully-qualified path, returns a sequence of paths which match the suffix of the input
/// path, in decreasing path-length order.
///
/// Example: `suffixes(".a.b.c.d") -> ["a.b.c.d", "b.c.d", "c.d", "d"]`
fn suffixes(fq_path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(fq_path), |path| {
        #[allow(unknown_lints, clippy::manual_split_once)]
        path.splitn(2, '.').nth(1).filter(|path| !path.is_empty())
    })
    .skip(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefixes() {
        assert_eq!(
            prefixes(".a.b.c.d").collect::<Vec<_>>(),
            vec![".a.b.c", ".a.b", ".a"],
        );
        assert_eq!(prefixes(".a").count(), 0);
        assert_eq!(prefixes(".").count(), 0);
    }

    #[test]
    fn test_suffixes() {
        assert_eq!(
            suffixes(".a.b.c.d").collect::<Vec<_>>(),
            vec!["a.b.c.d", "b.c.d", "c.d", "d"],
        );
        assert_eq!(suffixes(".a").collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(suffixes(".").collect::<Vec<_>>(), Vec::<&str>::new());
    }

    #[test]
    fn test_get_matches_sub_path() {
        for matcher in [".a.b.c.d", "a.b.c.d", "b.c.d", "c.d", "d", ".a.b", "."] {
            let mut path_map = PathMap::default();
            path_map.insert(matcher.to_owned(), 1);
            assert_eq!(Some(&1), path_map.get_first_field(".a.b.c", "d"));
        }

        let mut path_map = PathMap::default();
        assert_eq!(None, path_map.get_first_field(".a.b.c", "d"));
        path_map.insert(".a.b.c.e".to_owned(), 1);
        assert_eq!(None, path_map.get_first_field(".a.b.c", "d"));
    }

    #[test]
    fn test_get_best() {
        let mut path_map = PathMap::default();

        // worst is global
        path_map.insert(".".to_owned(), 1);
        assert_eq!(Some(&1), path_map.get_first_field(".a.b.c", "d"));

        // then prefix
        path_map.insert(".a.b".to_owned(), 2);
        assert_eq!(Some(&2), path_map.get_first_field(".a.b.c", "d"));

        // then suffix
        path_map.insert("c.d".to_owned(), 3);
        assert_eq!(Some(&3), path_map.get_first_field(".a.b.c", "d"));

        // best is full path
        path_map.insert(".a.b.c.d".to_owned(), 4);
        assert_eq!(Some(&4), path_map.get_first_field(".a.b.c", "d"));
    }

    #[test]
    fn test_get_keep_order() {
        let mut path_map = PathMap::default();
        path_map.insert(".a.b.c.d".to_owned(), 1);
        path_map.insert(".a.b.c.d".to_owned(), 2);
        assert_eq!(Some(&1), path_map.get_first_field(".a.b.c", "d"));
    }
}
