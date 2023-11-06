// Copyright (C) Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A simple storage vector implementation built on top of [Mapping].
//!
//! # Note
//!
//! This vector doesn't actually "own" any data.
//! Instead it is just a simple wrapper around the contract storage facilities.

use ink_primitives::Key;
use ink_storage_traits::{AutoKey, Packed, Storable, StorableHint, StorageKey};
use scale::EncodeLike;

use crate::{Lazy, Mapping};

/// A vector of values (elements) directly on contract storage.

/// # Important
///
/// [StorageVec] requires its own pre-defined storage key where to store values. By
/// default, the is automatically calculated using [`AutoKey`](crate::traits::AutoKey)
/// during compilation. However, anyone can specify a storage key using
/// [`ManualKey`](crate::traits::ManualKey). Specifying the storage key can be helpful for
/// upgradeable contracts or you want to be resistant to future changes of storage key
/// calculation strategy.
///
/// # Differences between `ink::prelude::vec::Vec` and [StorageVec]
///
/// Any `Vec<T>` will exhibit [Packed] storage layout; where
/// [StorageVec] stores each value under it's own storage key.
///
/// Hence, any read or write from or to a `Vec` on storage will load
/// or store _all_ of its elements.
///
/// This can be undesirable:
/// The cost of reading or writing a _single_ element grows linearly
/// corresponding to the number of elements in the vector (its length).
/// Additionally, the maximum capacity of the _whole_ vector is limited by
/// the size of the static buffer used during ABI encoding and decoding
/// (default 16KiB).
///
/// [StorageVec] on the other hand allows to access each element individually.
/// Thus, it can theoretically grow to infinite size.
/// However, we currently limit the length at 2 ^ 32 elements. In practice,
/// even if the vector elements are single bytes, it'll allow to store
/// more than 4GB data in blockchain storage.
///
/// # Caveats
///
/// Iteration is not providided. [StorageVec] is expected to be used to
/// store a lot or large values where iterating through elements would be
/// rather inefficient anyways.
///
/// The decision whether to use `Vec<T>` or [StorageVec] can be seen as an
/// optimization problem with several factors:
/// * How large you expect the vector to grow
/// * The size of individual elements being stored
/// * How frequentely reads, writes and iterations happen
///
/// For example, if a vector is expected to stay small but is frequently
/// iteratet over. Chooosing a `Vec<T>` instead of [StorageVec] will be
/// preferred as indiviudal storage reads are much more expensive as
/// opposed to retrieving and decoding the whole collections with a single
/// storage read.
///
/// # Storage Layout
///
/// At given [StorageKey] `K`, the length of the [StorageVec] is hold.
/// Each element `E` is then stored under a combination of the [StorageVec]
/// key `K` and the elements index.
///
/// Given [StorageVec] under key `K`, the storage key `E` of the `N`th
/// element is calcualted as follows:
///
/// `E = scale::Encode((K, N))`
///
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub struct StorageVec<V: Packed, KeyType: StorageKey = AutoKey> {
    len: Lazy<u32, KeyType>,
    len_cached: Option<u32>,
    elements: Mapping<u32, V, KeyType>,
}

impl<V, KeyType> Default for StorageVec<V, KeyType>
where
    V: Packed,
    KeyType: StorageKey,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<V, KeyType> Storable for StorageVec<V, KeyType>
where
    V: Packed,
    KeyType: StorageKey,
{
    #[inline]
    fn encode<T: scale::Output + ?Sized>(&self, _dest: &mut T) {}

    #[inline]
    fn decode<I: scale::Input>(_input: &mut I) -> Result<Self, scale::Error> {
        Ok(Default::default())
    }

    #[inline]
    fn encoded_size(&self) -> usize {
        0
    }
}

impl<V, Key, InnerKey> StorableHint<Key> for StorageVec<V, InnerKey>
where
    V: Packed,
    Key: StorageKey,
    InnerKey: StorageKey,
{
    type Type = StorageVec<V, Key>;
    type PreferredKey = InnerKey;
}

impl<V, KeyType> StorageKey for StorageVec<V, KeyType>
where
    V: Packed,
    KeyType: StorageKey,
{
    const KEY: Key = KeyType::KEY;
}

#[cfg(feature = "std")]
const _: () = {
    use crate::traits::StorageLayout;
    use ink_metadata::layout::{Layout, LayoutKey, RootLayout};

    impl<V, KeyType> StorageLayout for StorageVec<V, KeyType>
    where
        V: Packed + StorageLayout + scale_info::TypeInfo + 'static,
        KeyType: StorageKey + scale_info::TypeInfo + 'static,
    {
        fn layout(_: &Key) -> Layout {
            Layout::Root(RootLayout::new::<Self, _>(
                LayoutKey::from(&KeyType::KEY),
                <V as StorageLayout>::layout(&KeyType::KEY),
            ))
        }
    }
};

impl<V, KeyType> StorageVec<V, KeyType>
where
    V: Packed,
    KeyType: StorageKey,
{
    /// Creates a new empty `StorageVec`.
    pub const fn new() -> Self {
        Self {
            len: Lazy::new(),
            len_cached: None,
            elements: Mapping::new(),
        }
    }

    /// Returns the number of elements in the vector, also referred to as its length.
    ///
    /// The length is cached; subsequent calls (without writing to the vector) won't
    /// trigger additional storage reads.
    #[inline]
    pub fn len(&self) -> u32 {
        self.len_cached
            .unwrap_or_else(|| self.len.get().unwrap_or(u32::MIN))
    }

    fn set_len(&mut self, new_len: u32) {
        self.len.set(&new_len);
        self.len_cached = Some(new_len);
    }

    /// Returns `true` if the vector contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends an element to the back of the vector.
    ///
    /// # Panics
    ///
    /// * If the vector is at capacity (max. of 2 ^ 32 elements).
    /// * If the value overgrows the static buffer size.
    /// * If there was already a value at the current index.
    pub fn push<T>(&mut self, value: &T)
    where
        T: Storable + scale::EncodeLike<V>,
    {
        let slot = self.len();
        self.set_len(slot.checked_add(1).unwrap());

        assert!(self.elements.insert(slot, value).is_none());
    }

    /// Pops the last element from the vector and returns it.
    //
    /// Returns `None` if the vector is empty.
    ///
    /// # Panics
    ///
    /// * If the value overgrows the static buffer size.
    /// * If there is no value at the current index.
    pub fn pop(&mut self) -> Option<V> {
        let slot = self.len().checked_sub(1)?;

        self.set_len(slot);
        self.elements.take(slot).unwrap().into()
    }

    /// Access an element at given `index`.
    ///
    /// # Panics
    ///
    /// * If encoding the element exceeds the static buffer size.
    pub fn get(&self, index: u32) -> Option<V> {
        self.elements.get(index)
    }

    /// Set the `value` at given `index`.
    ///
    /// # Panics
    ///
    /// * If the index is out of bounds.
    /// * If decoding the element exceeds the static buffer size.
    pub fn set<T>(&mut self, index: u32, value: &T)
    where
        T: Storable + EncodeLike<V>,
    {
        assert!(index < self.len());

        let _ = self.elements.insert(index, value);
    }
}

impl<V, KeyType> ::core::fmt::Debug for StorageVec<V, KeyType>
where
    V: Packed,
    KeyType: StorageKey,
{
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("StorageVec")
            .field("key", &KeyType::KEY)
            .field("len", &self.len)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ManualKey;

    #[test]
    fn default_values() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let mut array: StorageVec<String> = StorageVec::new();

            assert_eq!(array.pop(), None);
            assert_eq!(array.len(), 0);

            Ok(())
        })
        .unwrap()
    }
    #[test]
    fn push_and_pop_work() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let mut array: StorageVec<String> = StorageVec::new();

            let value = "test".to_string();
            array.push(&value);
            assert_eq!(array.len(), 1);
            assert_eq!(array.pop(), Some(value));

            assert_eq!(array.len(), 0);
            assert_eq!(array.pop(), None);

            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn storage_keys_are_correct() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            const BASE: u32 = 123;
            let mut array: StorageVec<u8, ManualKey<BASE>> = StorageVec::new();

            let expected_value = 127;
            array.push(&expected_value);

            let actual_length = ink_env::get_contract_storage::<_, u32>(&BASE);
            assert_eq!(actual_length, Ok(Some(1)));

            let actual_value = ink_env::get_contract_storage::<_, u8>(&(BASE, 0u32));
            assert_eq!(actual_value, Ok(Some(expected_value)));

            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn push_and_pop_work_for_two_vecs_with_same_manual_key() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let expected_value = 255;

            let mut array: StorageVec<u8, ManualKey<{ u32::MIN }>> = StorageVec::new();
            array.push(&expected_value);

            let mut array2: StorageVec<u8, ManualKey<{ u32::MIN }>> = StorageVec::new();
            assert_eq!(array2.pop(), Some(expected_value));

            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn set_and_get_work() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            let mut array: StorageVec<String> = StorageVec::new();

            let value = "test".to_string();
            array.push(&value);
            assert_eq!(array.get(0), Some(value));
            assert_eq!(array.len(), 1);

            let replaced_value = "foo".to_string();
            array.set(0, &replaced_value);
            assert_eq!(array.get(0), Some(replaced_value));

            Ok(())
        })
        .unwrap()
    }

    #[test]
    #[should_panic]
    fn set_panics_on_oob() {
        ink_env::test::run_test::<ink_env::DefaultEnvironment, _>(|_| {
            StorageVec::<u8>::new().set(0, &0);

            Ok(())
        })
        .unwrap()
    }
}