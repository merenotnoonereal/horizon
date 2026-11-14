//! `chunk_data_load` and `chunk_data_save`: the disk hooks run each mod through the disk runtime's
//! `invoke` and stop at the first failed job. Stand-ins for the restricted host and the store.
mod stubs {
    pub(super) trait ChunkDiskHost {}
    #[derive(Clone, Debug, Default)]
    pub(super) struct ChunkDiskFailure {
        pub mod_id: String,
    }
    #[derive(Default)]
    pub(super) struct ChunkDiskData {
        failure: Option<ChunkDiskFailure>,
    }
    impl ChunkDiskData {
        pub(super) fn failure(&self) -> Option<&ChunkDiskFailure> {
            self.failure.as_ref()
        }
        pub(super) fn refuse(&mut self, error: ChunkDiskFailure) {
            self.failure.get_or_insert(error);
        }
        pub(super) fn attribute_failure(&mut self, error: ChunkDiskFailure) {
            self.failure = Some(error);
        }
    }
    pub(super) mod chunk_disk {
        pub(crate) fn invoke(
            _: &str,
            call: impl FnOnce(),
        ) -> Result<(), super::ChunkDiskFailure> {
            call();
            Ok(())
        }
    }
}
use stubs::chunk_disk;
