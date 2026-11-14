//! Link-refusing directory trees, staged publication and clean tracked-source copies.

mod source;
mod tree;

pub(crate) use source::{finish_staged_operation, publish_staged_file, with_clean_source_copy};
pub(crate) use tree::{
    canonical_existing_real_directory, metadata_is_real_directory, open_real_directory,
    open_real_file, prepare_real_child, prepare_real_directory_tree, require_direct_file,
    require_direct_file_if_present,
};
