#![no_main]

use libfuzzer_sys::fuzz_target;
use punctra_persistence_fuzz::exercise_terrain_persisted_bytes;

fuzz_target!(|input: &[u8]| {
    exercise_terrain_persisted_bytes(input);
});
