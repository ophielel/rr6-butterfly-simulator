//! Helpers shared by the crate's tests.  They load the generated data library
//! so the tests fail loudly when the pipeline has not been run.

use crate::effects::MechanicsBook;
use crate::library::Library;
use std::path::PathBuf;

pub fn data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("data")
}

pub fn test_library() -> (Library, MechanicsBook) {
    let root = data_root();
    assert!(
        root.join("identities").join("10110.json").exists(),
        "data library missing - run `python3 tools/build_library.py` (looked in {})",
        root.display()
    );
    let library = Library::load(&root).expect("library loads");
    let mechanics =
        MechanicsBook::load(&root.join("mechanics").join("effects.json")).expect("mechanics load");
    (library, mechanics)
}
