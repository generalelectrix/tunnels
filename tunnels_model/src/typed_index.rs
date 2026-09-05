/// Generate `Index`/`IndexMut` and `From` impls for a newtype index.
///
/// Replaces the `typed_index_derive` crate (which depends on ancient `syn 0.14`)
/// with an equivalent declarative macro, minus unused arithmetic impls.
macro_rules! typed_index {
    ($idx:ident, $target:ty) => {
        impl std::ops::Index<$idx> for [$target] {
            type Output = $target;
            fn index(&self, index: $idx) -> &$target {
                &self[index.0]
            }
        }

        impl std::ops::IndexMut<$idx> for [$target] {
            fn index_mut(&mut self, index: $idx) -> &mut $target {
                &mut self[index.0]
            }
        }

        impl std::ops::Index<$idx> for Vec<$target> {
            type Output = $target;
            fn index(&self, index: $idx) -> &$target {
                &self.as_slice()[index]
            }
        }

        impl std::ops::IndexMut<$idx> for Vec<$target> {
            fn index_mut(&mut self, index: $idx) -> &mut $target {
                &mut self.as_mut_slice()[index]
            }
        }

        impl From<usize> for $idx {
            fn from(val: usize) -> $idx {
                $idx(val)
            }
        }

        impl From<$idx> for usize {
            fn from(val: $idx) -> usize {
                val.0
            }
        }
    };
}

pub(crate) use typed_index;
