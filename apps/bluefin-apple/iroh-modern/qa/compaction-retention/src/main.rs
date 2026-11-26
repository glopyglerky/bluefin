//! Isolated, single-threaded requested-allocation regression for the selected assembler.
//! The source is included verbatim; this binary is never linked into a client.
#![allow(dead_code)]
extern crate alloc;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);
struct Meter;

// Delegate every operation and alignment unchanged to the system allocator.
// Count only successful requested bytes, not RSS or allocator page reservations.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(size, Ordering::Relaxed);
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Meter = Meter;

#[path = "../../../vendor/noq-proto-1.4.0/src/range_set/mod.rs"]
mod range_set;

mod connection {
    mod assembler {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../vendor/noq-proto-1.4.0/src/connection/assembler.rs"
        ));

        fn packet(size: usize) -> Bytes {
            Bytes::from(vec![0x5a; size + 64]).slice(..size)
        }
        fn invariants(assembler: &Assembler) {
            assert_eq!(
                assembler.data.iter().map(|c| c.bytes.len()).sum::<usize>(),
                assembler.buffered
            );
            assert_eq!(
                assembler
                    .data
                    .iter()
                    .map(|c| c.allocation_size)
                    .sum::<usize>(),
                assembler.allocated
            );
            assert!(assembler.data.len() <= COMPACT_THRESHOLD + 1);
            for (a, b) in assembler.data.iter().zip(assembler.data.iter().skip(1)) {
                assert!(a <= b);
            }
        }

        pub fn run() {
            const WINDOW: usize = 8 * 1024 * 1024;
            let live = || crate::LIVE.load(std::sync::atomic::Ordering::Relaxed);
            let baseline = live();
            let mut assembler = Assembler::new();
            let mut offset = 1u64;
            // Withhold byte zero. All offsets fit the actual 8 MiB receive window.
            for _ in 0..3584 {
                assembler.insert(offset, packet(1024), 1088).unwrap();
                offset += 1024;
            }
            let first = offset + 1;
            for island in 0..512u64 {
                for part in 0..2 {
                    assembler
                        .insert(first + island * 8193 + part * 1024, packet(1024), 1088)
                        .unwrap();
                }
            }
            for _ in 0..2050 {
                assembler.insert(1, packet(1), 65).unwrap();
            }
            for cycle in 0..64u64 {
                // Growing one small island retains its older slice while duplicates
                // force another compaction. No returned Chunk is retained.
                for part in 2..8 {
                    assembler
                        .insert(first + cycle * 8193 + part * 1024, packet(1024), 1088)
                        .unwrap();
                }
                for _ in 0..2050 {
                    assembler.insert(1, packet(1), 65).unwrap();
                }
                invariants(&assembler);
                assert!(assembler.end < WINDOW as u64);
                assert!(assembler.read(1, true).is_none());
            }
            let requested = live() - baseline;
            let buffered = assembler.buffered;
            let estimated = assembler.allocated;
            let end = assembler.end;
            // A deliberately generous regression guard, not a protocol quota.
            assert!(
                requested <= 4 * WINDOW,
                "retained requested bytes {requested} exceed four receive windows; buffered={buffered}, estimated={estimated}"
            );
            assembler.ensure_ordering(false).unwrap();
            let mut previous_end = 0;
            let mut consumed = 0;
            while let Some(chunk) = assembler.read(1733, false) {
                assert!(chunk.offset >= previous_end);
                assert!(chunk.bytes.iter().all(|byte| *byte == 0x5a));
                previous_end = chunk.offset + chunk.bytes.len() as u64;
                consumed += chunk.bytes.len();
            }
            assert_eq!(consumed, (3584 + 512 * 2 + 64 * 6) * 1024);
            assert_eq!((assembler.buffered, assembler.allocated), (0, 0));
            drop(assembler);
            assert_eq!(live(), baseline, "Assembler drop retained an allocation");
            println!(
                "{{\"retainedRequestedBytes\":{requested},\"bufferedEstimate\":{buffered},\"allocatedEstimate\":{estimated},\"streamEnd\":{end},\"afterDropDelta\":0}}"
            );
        }
    }
    pub fn run() {
        assembler::run();
    }
}

fn main() {
    // Warm stdout before the baseline: its process-lifetime buffer is not assembler ownership.
    println!("Bluefin selected-assembler retained-allocation regression");
    connection::run();
}
