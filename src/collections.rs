//! Collection types: the `alloc` collections plus hash maps and sets
//! (hashbrown, keyed with a per-process random SipHash key).

pub use alloc_crate::collections::*;

pub use self::hash_map::HashMap;
pub use self::hash_set::HashSet;

pub mod hash_map {
    use core::borrow::Borrow;
    use core::fmt;
    use core::hash::{BuildHasher, Hash, Hasher};
    use core::ops::{Deref, DerefMut, Index};
    use core::sync::atomic::{AtomicU64, Ordering};

    pub use hashbrown::hash_map::{
        Drain, Entry, IntoIter, IntoKeys, IntoValues, Iter, IterMut, Keys, OccupiedEntry, VacantEntry, Values,
        ValuesMut,
    };

    /// The default hasher: SipHash with keys from [`RandomState`].
    #[derive(Clone, Debug)]
    #[allow(deprecated)]
    pub struct DefaultHasher(core::hash::SipHasher);

    #[allow(deprecated)]
    impl DefaultHasher {
        pub fn new() -> DefaultHasher {
            DefaultHasher(core::hash::SipHasher::new_with_keys(0, 0))
        }
    }

    impl Default for DefaultHasher {
        fn default() -> DefaultHasher {
            DefaultHasher::new()
        }
    }

    impl Hasher for DefaultHasher {
        fn write(&mut self, msg: &[u8]) {
            self.0.write(msg)
        }
        fn finish(&self) -> u64 {
            self.0.finish()
        }
    }

    /// Builds [`DefaultHasher`]s keyed randomly once per process (via
    /// `getrandom`), with the key incremented for each new `RandomState`.
    #[derive(Clone)]
    pub struct RandomState {
        k0: u64,
        k1: u64,
    }

    impl RandomState {
        pub fn new() -> RandomState {
            static K0: AtomicU64 = AtomicU64::new(0);
            static K1: AtomicU64 = AtomicU64::new(0);
            static INIT: crate::sync::Once = crate::sync::Once::new();
            INIT.call_once(|| {
                let mut buf = [0u8; 16];
                if unsafe { crate::sys::linux::getrandom(buf.as_mut_ptr(), buf.len(), 0) } != Ok(16) {
                    // fall back to the 16 random bytes the kernel put in auxv (AT_RANDOM)
                    if let Some(p) = crate::rt::auxv(25) {
                        unsafe { core::ptr::copy_nonoverlapping(p as *const u8, buf.as_mut_ptr(), 16) };
                    }
                }
                K0.store(u64::from_ne_bytes(buf[..8].try_into().unwrap()), Ordering::Relaxed);
                K1.store(u64::from_ne_bytes(buf[8..].try_into().unwrap()), Ordering::Relaxed);
            });
            RandomState { k0: K0.fetch_add(1, Ordering::Relaxed), k1: K1.load(Ordering::Relaxed) }
        }
    }

    impl Default for RandomState {
        fn default() -> RandomState {
            RandomState::new()
        }
    }

    impl BuildHasher for RandomState {
        type Hasher = DefaultHasher;
        #[allow(deprecated)]
        fn build_hasher(&self) -> DefaultHasher {
            DefaultHasher(core::hash::SipHasher::new_with_keys(self.k0, self.k1))
        }
    }

    impl fmt::Debug for RandomState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("RandomState").finish_non_exhaustive()
        }
    }

    /// A hash map; derefs to [`hashbrown::HashMap`] for the full method set.
    pub struct HashMap<K, V, S = RandomState> {
        base: hashbrown::HashMap<K, V, S>,
    }

    impl<K, V> HashMap<K, V, RandomState> {
        pub fn new() -> HashMap<K, V, RandomState> {
            HashMap::with_hasher(RandomState::new())
        }

        pub fn with_capacity(capacity: usize) -> HashMap<K, V, RandomState> {
            HashMap::with_capacity_and_hasher(capacity, RandomState::new())
        }
    }

    impl<K, V, S> HashMap<K, V, S> {
        pub const fn with_hasher(hash_builder: S) -> HashMap<K, V, S> {
            HashMap { base: hashbrown::HashMap::with_hasher(hash_builder) }
        }

        pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> HashMap<K, V, S> {
            HashMap { base: hashbrown::HashMap::with_capacity_and_hasher(capacity, hasher) }
        }
    }

    impl<K, V, S> Deref for HashMap<K, V, S> {
        type Target = hashbrown::HashMap<K, V, S>;
        fn deref(&self) -> &Self::Target {
            &self.base
        }
    }

    impl<K, V, S> DerefMut for HashMap<K, V, S> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.base
        }
    }

    impl<K: Clone, V: Clone, S: Clone> Clone for HashMap<K, V, S> {
        fn clone(&self) -> Self {
            HashMap { base: self.base.clone() }
        }
    }

    impl<K: fmt::Debug, V: fmt::Debug, S> fmt::Debug for HashMap<K, V, S> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_map().entries(self.base.iter()).finish()
        }
    }

    impl<K, V, S: Default> Default for HashMap<K, V, S> {
        fn default() -> Self {
            HashMap::with_hasher(S::default())
        }
    }

    impl<K: Eq + Hash, V: PartialEq, S: BuildHasher> PartialEq for HashMap<K, V, S> {
        fn eq(&self, other: &Self) -> bool {
            self.base == other.base
        }
    }

    impl<K: Eq + Hash, V: Eq, S: BuildHasher> Eq for HashMap<K, V, S> {}

    impl<K, Q: ?Sized, V, S> Index<&Q> for HashMap<K, V, S>
    where
        K: Eq + Hash + Borrow<Q>,
        Q: Eq + Hash,
        S: BuildHasher,
    {
        type Output = V;
        fn index(&self, key: &Q) -> &V {
            self.base.get(key).expect("no entry found for key")
        }
    }

    impl<K: Eq + Hash, V, S: BuildHasher + Default> FromIterator<(K, V)> for HashMap<K, V, S> {
        fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
            let mut map = HashMap::with_hasher(S::default());
            map.extend(iter);
            map
        }
    }

    impl<K: Eq + Hash, V, S: BuildHasher> Extend<(K, V)> for HashMap<K, V, S> {
        fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
            self.base.extend(iter)
        }
    }

    impl<'a, K: Eq + Hash + Copy, V: Copy, S: BuildHasher> Extend<(&'a K, &'a V)> for HashMap<K, V, S> {
        fn extend<T: IntoIterator<Item = (&'a K, &'a V)>>(&mut self, iter: T) {
            self.base.extend(iter)
        }
    }

    impl<K: Eq + Hash, V, const N: usize> From<[(K, V); N]> for HashMap<K, V, RandomState> {
        fn from(arr: [(K, V); N]) -> Self {
            HashMap::from_iter(arr)
        }
    }

    impl<K, V, S> IntoIterator for HashMap<K, V, S> {
        type Item = (K, V);
        type IntoIter = IntoIter<K, V>;
        fn into_iter(self) -> IntoIter<K, V> {
            self.base.into_iter()
        }
    }

    impl<'a, K, V, S> IntoIterator for &'a HashMap<K, V, S> {
        type Item = (&'a K, &'a V);
        type IntoIter = Iter<'a, K, V>;
        fn into_iter(self) -> Iter<'a, K, V> {
            self.base.iter()
        }
    }

    impl<'a, K, V, S> IntoIterator for &'a mut HashMap<K, V, S> {
        type Item = (&'a K, &'a mut V);
        type IntoIter = IterMut<'a, K, V>;
        fn into_iter(self) -> IterMut<'a, K, V> {
            self.base.iter_mut()
        }
    }
}

pub mod hash_set {
    use core::fmt;
    use core::hash::{BuildHasher, Hash};
    use core::ops::{BitAnd, BitOr, Deref, DerefMut, Sub};

    pub use hashbrown::hash_set::{Difference, Drain, Intersection, IntoIter, Iter, SymmetricDifference, Union};

    use super::hash_map::RandomState;

    /// A hash set; derefs to [`hashbrown::HashSet`] for the full method set.
    pub struct HashSet<T, S = RandomState> {
        base: hashbrown::HashSet<T, S>,
    }

    impl<T> HashSet<T, RandomState> {
        pub fn new() -> HashSet<T, RandomState> {
            HashSet::with_hasher(RandomState::new())
        }

        pub fn with_capacity(capacity: usize) -> HashSet<T, RandomState> {
            HashSet::with_capacity_and_hasher(capacity, RandomState::new())
        }
    }

    impl<T, S> HashSet<T, S> {
        pub const fn with_hasher(hasher: S) -> HashSet<T, S> {
            HashSet { base: hashbrown::HashSet::with_hasher(hasher) }
        }

        pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> HashSet<T, S> {
            HashSet { base: hashbrown::HashSet::with_capacity_and_hasher(capacity, hasher) }
        }
    }

    impl<T, S> Deref for HashSet<T, S> {
        type Target = hashbrown::HashSet<T, S>;
        fn deref(&self) -> &Self::Target {
            &self.base
        }
    }

    impl<T, S> DerefMut for HashSet<T, S> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.base
        }
    }

    impl<T: Clone, S: Clone> Clone for HashSet<T, S> {
        fn clone(&self) -> Self {
            HashSet { base: self.base.clone() }
        }
    }

    impl<T: fmt::Debug, S> fmt::Debug for HashSet<T, S> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_set().entries(self.base.iter()).finish()
        }
    }

    impl<T, S: Default> Default for HashSet<T, S> {
        fn default() -> Self {
            HashSet::with_hasher(S::default())
        }
    }

    impl<T: Eq + Hash, S: BuildHasher> PartialEq for HashSet<T, S> {
        fn eq(&self, other: &Self) -> bool {
            self.base == other.base
        }
    }

    impl<T: Eq + Hash, S: BuildHasher> Eq for HashSet<T, S> {}

    impl<T: Eq + Hash, S: BuildHasher + Default> FromIterator<T> for HashSet<T, S> {
        fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
            let mut set = HashSet::with_hasher(S::default());
            set.extend(iter);
            set
        }
    }

    impl<T: Eq + Hash, S: BuildHasher> Extend<T> for HashSet<T, S> {
        fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
            self.base.extend(iter)
        }
    }

    impl<'a, T: 'a + Eq + Hash + Copy, S: BuildHasher> Extend<&'a T> for HashSet<T, S> {
        fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
            self.base.extend(iter)
        }
    }

    impl<T: Eq + Hash, const N: usize> From<[T; N]> for HashSet<T, RandomState> {
        fn from(arr: [T; N]) -> Self {
            HashSet::from_iter(arr)
        }
    }

    impl<T, S> IntoIterator for HashSet<T, S> {
        type Item = T;
        type IntoIter = IntoIter<T>;
        fn into_iter(self) -> IntoIter<T> {
            self.base.into_iter()
        }
    }

    impl<'a, T, S> IntoIterator for &'a HashSet<T, S> {
        type Item = &'a T;
        type IntoIter = Iter<'a, T>;
        fn into_iter(self) -> Iter<'a, T> {
            self.base.iter()
        }
    }

    macro_rules! set_op {
        ($tr:ident, $m:ident, $op:tt) => {
            impl<T: Eq + Hash + Clone, S: BuildHasher + Default> $tr<&HashSet<T, S>> for &HashSet<T, S> {
                type Output = HashSet<T, S>;
                fn $m(self, rhs: &HashSet<T, S>) -> HashSet<T, S> {
                    HashSet { base: &self.base $op &rhs.base }
                }
            }
        };
    }

    set_op!(BitOr, bitor, |);
    set_op!(BitAnd, bitand, &);
    set_op!(Sub, sub, -);
}
