//! Refreshes of host copies of guarded view memory while the title's
//! threads write it (fork issue #1083): what a flush costs, and how long
//! the flush and the writers wait for the guards' lock and the journal's.
//!
//! The title memory is a placeholder reservation with a pagefile section
//! mapped into it 64 KiB block by block, as the runtime maps physical
//! pages; host copies (`Shadow`s over plain host memory) cover it. A
//! round: the writers dirty some blocks (the frame before), then the
//! flush refreshes every copy while the writers keep writing random
//! blocks, each write into a block the flush has guarded again faulting
//! into the guards' handler.

use super::Shadow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::*;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MEM_PRESERVE_PLACEHOLDER, MEM_RELEASE, MEM_REPLACE_PLACEHOLDER,
    MEM_RESERVE, MEM_RESERVE_PLACEHOLDER, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile3,
    PAGE_NOACCESS, PAGE_READWRITE, UnmapViewOfFile2, VirtualAlloc2, VirtualFree,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const BLOCK: usize = 64 << 10;

/// A plain buffer on the software adapter: a copy's `buffer`, which the
/// refresh never touches.
fn warp_buffer() -> Option<ID3D12Resource> {
    // SAFETY: plain DXGI and D3D12 creation calls on the software adapter.
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
        let adapter: IDXGIAdapter = factory.EnumWarpAdapter().ok()?;
        let mut device: Option<ID3D12Device> = None;
        D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device).ok()?;
        let device = device?;
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: 64 << 10,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        let mut buffer: Option<ID3D12Resource> = None;
        device
            .CreateCommittedResource(
                &heap,
                D3D12_HEAP_FLAG_NONE,
                &desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut buffer,
            )
            .ok()?;
        buffer
    }
}

/// `blocks` blocks of title view memory: a placeholder with a pagefile
/// section mapped into it a block at a time.
struct Title {
    at: usize,
    blocks: usize,
    section: windows_sys::Win32::Foundation::HANDLE,
    views: Vec<MEMORY_MAPPED_VIEW_ADDRESS>,
}

impl Title {
    fn new(blocks: usize) -> Title {
        // SAFETY: a fresh placeholder and pagefile section, split and
        // mapped block by block; all ours until dropped.
        unsafe {
            let at = VirtualAlloc2(
                GetCurrentProcess(),
                std::ptr::null(),
                blocks * BLOCK,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            ) as usize;
            assert_ne!(at, 0);
            let bytes = (blocks * BLOCK) as u64;
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                (bytes >> 32) as u32,
                bytes as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            let mut views = Vec::with_capacity(blocks);
            for block in 0..blocks {
                let place = at + block * BLOCK;
                if block + 1 < blocks {
                    VirtualFree(
                        place as *mut _,
                        BLOCK,
                        MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                    );
                }
                let view = MapViewOfFile3(
                    section,
                    GetCurrentProcess(),
                    place as *const _,
                    (block * BLOCK) as u64,
                    BLOCK,
                    MEM_REPLACE_PLACEHOLDER,
                    PAGE_READWRITE,
                    std::ptr::null_mut(),
                    0,
                );
                assert_eq!(view.Value as usize, place);
                views.push(view);
            }
            crate::watch::title_memory_remapped(at, blocks * BLOCK);
            Title {
                at,
                blocks,
                section,
                views,
            }
        }
    }

    fn len(&self) -> usize {
        self.blocks * BLOCK
    }
}

impl Drop for Title {
    fn drop(&mut self) {
        crate::watch::title_memory_remapped(self.at, self.len());
        // SAFETY: our views, placeholder and section.
        unsafe {
            for &view in &self.views {
                UnmapViewOfFile2(GetCurrentProcess(), view, MEM_PRESERVE_PLACEHOLDER);
            }
            for block in 0..self.blocks {
                VirtualFree((self.at + block * BLOCK) as *mut _, 0, MEM_RELEASE);
            }
            windows_sys::Win32::Foundation::CloseHandle(self.section);
        }
        crate::watch::title_memory_remapped(self.at, self.len());
    }
}

/// A host copy over `[address, address + len)` of title memory, in host
/// memory of its own (`side`, kept alive beside it).
struct Copy {
    shadow: Arc<Shadow>,
    side: Box<[u8]>,
}

fn copies(buffer: &ID3D12Resource, title: &Title, span: usize, overlap: bool) -> Vec<Copy> {
    let mut ranges: Vec<(usize, usize)> = (0..title.len() / span)
        .map(|i| (title.at + i * span, span))
        .collect();
    if overlap {
        // Copies of the same memory that cross the others' edges, as a
        // texture inside a buffer: the journal tells each what another's
        // asking found.
        ranges.extend((0..title.len() / span - 1).map(|i| (title.at + i * span + span / 2, span)));
    }
    ranges
        .into_iter()
        .map(|(address, len)| {
            let mut side = vec![0u8; len].into_boxed_slice();
            let shadow = Arc::new(Shadow::new(
                buffer.clone(),
                side.as_mut_ptr(),
                0,
                address as u64,
                len as u64,
            ));
            Copy { shadow, side }
        })
        .collect()
}

/// One flush: every copy refreshed whole, on this thread or (`wide`) on
/// the helper threads. The bytes copied.
fn flush(copies: &[Copy], wide: bool) -> u64 {
    let bytes = Arc::new(AtomicU64::new(0));
    if wide {
        let jobs: Vec<Box<dyn FnOnce() + Send>> = copies
            .iter()
            .map(|c| {
                let (shadow, bytes) = (c.shadow.clone(), bytes.clone());
                Box::new(move || {
                    let n = shadow.update(shadow.start as usize, shadow.len as usize);
                    bytes.fetch_add(n as u64, Relaxed);
                }) as Box<dyn FnOnce() + Send>
            })
            .collect();
        super::wide_run(jobs);
    } else {
        for c in copies {
            let n = c
                .shadow
                .update(c.shadow.start as usize, c.shadow.len as usize);
            bytes.fetch_add(n as u64, Relaxed);
        }
    }
    bytes.load(Relaxed)
}

/// Sets its flag when dropped.
struct Stop<'a>(&'a AtomicBool);

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Relaxed);
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The contention benchmark (`cargo test -p victory-gpu --release
/// guard_contention -- --ignored --nocapture`). Sizes by the environment:
/// `BENCH_BLOCKS` (title memory in 64 KiB blocks, 4,096 = 256 MB),
/// `BENCH_SPAN` (blocks per copy, 32), `BENCH_DIRTY` (blocks written
/// before each flush, 1,400 = about 89 MB), `BENCH_WRITERS` (title
/// threads, 4), `BENCH_GAP_US` (a writer's pause between writes during
/// the flush, 20), `BENCH_ROUNDS` (11), `BENCH_WIDE` (1: the flush on the
/// helper threads).
#[test]
#[ignore = "benchmark: run by hand on a quiet machine"]
fn guard_contention() {
    let Some(buffer) = warp_buffer() else {
        return;
    };
    let blocks = env("BENCH_BLOCKS", 4096);
    let span = env("BENCH_SPAN", 32) * BLOCK;
    let dirty = env("BENCH_DIRTY", 1400).min(blocks);
    let writers = env("BENCH_WRITERS", 4);
    let gap = env("BENCH_GAP_US", 20) as u64;
    let rounds = env("BENCH_ROUNDS", 11);
    let wide = env("BENCH_WIDE", 0) == 1;
    let title = Title::new(blocks);
    let copies = copies(&buffer, &title, span, false);
    // The first flush takes everything and guards it.
    flush(&copies, false);
    let (at, len) = (title.at, title.len());
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut rows: Vec<[f64; 9]> = Vec::new();
    for round in 0..rounds {
        // The frame before: `dirty` distinct blocks written, by the writers.
        let mut order: Vec<usize> = (0..blocks).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.next() as usize % (i + 1));
        }
        let picked = &order[..dirty];
        std::thread::scope(|s| {
            for part in picked.chunks(dirty.div_ceil(writers).max(1)) {
                s.spawn(move || {
                    for &block in part {
                        // SAFETY: inside the title memory, mapped read-write
                        // or guarded (the handler opens it).
                        unsafe {
                            ((at + block * BLOCK + 8 * (block % 1000)) as *mut u64)
                                .write_volatile(round as u64)
                        };
                    }
                });
            }
        });
        // The flush, with the writers writing on.
        let stop = AtomicBool::new(false);
        let w0 = crate::watch::WAITS
            .each_ref()
            .map(|w| w.each_ref().map(|c| c.load(Relaxed)));
        let g0 = crate::watch::GUARD_WORK.each_ref().map(|c| c.load(Relaxed));
        let (flush_ms, bytes, mine, theirs, writes, write_us) = std::thread::scope(|s| {
            let handles: Vec<_> = (0..writers)
                .map(|k| {
                    let stop = &stop;
                    s.spawn(move || {
                        let mut rng = Rng(0x1234_5678 + k as u64 * 77 + round as u64);
                        let before = crate::watch::waited_here();
                        let (mut n, mut spent) = (0u64, 0u64);
                        while !stop.load(Relaxed) {
                            let offset = rng.next() as usize % (len / 8) * 8;
                            let started = std::time::Instant::now();
                            // SAFETY: as above.
                            unsafe { ((at + offset) as *mut u64).write_volatile(rng.0) };
                            spent += started.elapsed().as_nanos() as u64;
                            n += 1;
                            let until =
                                std::time::Instant::now() + std::time::Duration::from_micros(gap);
                            while std::time::Instant::now() < until {
                                std::hint::spin_loop();
                            }
                        }
                        let after = crate::watch::waited_here();
                        (after[0] - before[0] + after[1] - before[1], n, spent)
                    })
                })
                .collect();
            let before = crate::watch::waited_here();
            let started = std::time::Instant::now();
            let bytes = flush(&copies, wide);
            let flush_ms = started.elapsed().as_secs_f64() * 1e3;
            let after = crate::watch::waited_here();
            stop.store(true, Relaxed);
            let (mut theirs, mut writes, mut write_us) = (0, 0, 0);
            for h in handles {
                let (w, n, ns) = h.join().unwrap();
                theirs += w;
                writes += n;
                write_us += ns / 1000;
            }
            let mine = after[0] - before[0] + after[1] - before[1];
            (flush_ms, bytes, mine, theirs, writes, write_us)
        });
        let w1 = crate::watch::WAITS
            .each_ref()
            .map(|w| w.each_ref().map(|c| c.load(Relaxed)));
        let g1 = crate::watch::GUARD_WORK.each_ref().map(|c| c.load(Relaxed));
        let all_waits = (w1[0][1] - w0[0][1] + w1[1][1] - w0[1][1]) as f64 / 1e3;
        let row = [
            flush_ms,
            bytes as f64 / 1e6,
            mine as f64 / 1e3,
            theirs as f64 / 1e3,
            all_waits,
            (g1[1] - g0[1]) as f64 / 1e3,
            (g1[2] - g0[2]) as f64,
            (g1[3] - g0[3]) as f64 / 1e3,
            if writes == 0 {
                0.0
            } else {
                write_us as f64 / writes as f64
            },
        ];
        println!(
            "round {round:2}: flush {:6.2} ms, {:5.1} MB copied; lock waits: flush thread {:5.2} ms, writers {:6.2} ms, all threads {:6.2} ms; guards' lock held {:6.2} ms; {:5.0} re-guard calls {:5.2} ms; a writer's write {:5.2} us on average",
            row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7], row[8]
        );
        rows.push(row);
    }
    let m = |i: usize| median(rows.iter().map(|r| r[i]).collect());
    println!(
        "MEDIAN of {rounds} (blocks {blocks}, span {} blocks, dirty {dirty}, writers {writers}, gap {gap} us, wide {wide}): flush {:.2} ms, {:.1} MB; waits flush thread {:.2} ms, writers {:.2} ms, all {:.2} ms; held {:.2} ms; re-guard calls {:.0} ({:.2} ms); write {:.2} us",
        span / BLOCK,
        m(0),
        m(1),
        m(2),
        m(3),
        m(4),
        m(5),
        m(6),
        m(7),
        m(8)
    );
    drop(copies);
}

/// Writers and flushes interleaved, on one thread and on the helper
/// threads, with copies that overlap: no write is lost (a refresh that
/// began after a write holds it, or a later one), none is made up or
/// taken back (a copy never holds a value newer than was written, nor
/// older than it held after the flush before), and once the writers stop
/// one flush makes every copy equal to title memory.
#[test]
fn flushes_under_writes_lose_nothing() {
    // Askings by the thousand: not beside the tests that count the light
    // refresh's (`VICTORY_LIGHT_REFRESH` set for a run of all tests).
    let _turn = crate::watch::tests::light_turn();
    let Some(buffer) = warp_buffer() else {
        return;
    };
    const BLOCKS: usize = 96;
    const WRITERS: usize = 3;
    // Each writer counts up in its own words, spread over every block.
    const SLOTS: usize = 64;
    let title = Title::new(BLOCKS);
    let (at, len) = (title.at, title.len());
    let slot = |w: usize, i: usize| -> usize {
        // Block `i` (of the writer's slots), at a place of the writer's.
        let block = (i * 37 + w * 11) % BLOCKS;
        at + block * BLOCK + (w * SLOTS + i) * 8 * 13 % (BLOCK - 8) / 8 * 8
    };
    let written: Vec<Vec<AtomicU64>> = (0..WRITERS)
        .map(|_| (0..SLOTS).map(|_| AtomicU64::new(0)).collect())
        .collect();
    // Each writer's number, told before its store: no word of its holds
    // more. Both rounds count on from where the one before stopped.
    let latest: Vec<AtomicU64> = (0..WRITERS).map(|_| AtomicU64::new(0)).collect();
    for (wide, outside) in [(false, false), (true, false), (false, true), (true, true)] {
        crate::watch::GUARD_OUTSIDE_TEST.store(if outside { 2 } else { 1 }, Relaxed);
        let copies = copies(&buffer, &title, 8 * BLOCK, true);
        flush(&copies, false);
        let stop = AtomicBool::new(false);
        // What each copy held at each slot after the flush before.
        let mut held: Vec<Vec<u64>> = vec![vec![0; WRITERS * SLOTS]; copies.len()];
        std::thread::scope(|s| {
            for (w, mine) in written.iter().enumerate() {
                let (stop, latest) = (&stop, &latest[w]);
                s.spawn(move || {
                    let mut n = latest.load(Relaxed);
                    let mut rng = Rng(0xfeed + w as u64);
                    while !stop.load(Relaxed) {
                        n += 1;
                        let i = rng.next() as usize % SLOTS;
                        latest.store(n, std::sync::atomic::Ordering::Release);
                        // SAFETY: inside the title memory (the handler opens
                        // a guarded block).
                        unsafe { (slot(w, i) as *mut u64).write_volatile(n) };
                        // Published after the store: a flush that reads this
                        // number before it begins finds the store done.
                        mine[i].store(n, std::sync::atomic::Ordering::Release);
                        if n % 64 == 0 {
                            std::thread::yield_now();
                        }
                    }
                });
            }
            // The writers stop however this ends (a failed check too: the
            // scope waits for them).
            let _stop = Stop(&stop);
            for _ in 0..100 {
                let floor: Vec<u64> = written
                    .iter()
                    .flat_map(|w| {
                        w.iter()
                            .map(|v| v.load(std::sync::atomic::Ordering::Acquire))
                    })
                    .collect();
                flush(&copies, wide);
                let ceiling: Vec<u64> = latest
                    .iter()
                    .map(|v| v.load(std::sync::atomic::Ordering::Acquire))
                    .collect();
                for (c, copy) in copies.iter().enumerate() {
                    let start = copy.shadow.start as usize;
                    for w in 0..WRITERS {
                        for i in 0..SLOTS {
                            let address = slot(w, i);
                            if address < start || address + 8 > start + copy.side.len() {
                                continue;
                            }
                            let k = w * SLOTS + i;
                            // SAFETY: inside the copy's side, written by the
                            // flush that has returned.
                            let value = unsafe {
                                (copy.side.as_ptr().add(address - start) as *const u64)
                                    .read_unaligned()
                            };
                            assert!(
                                value >= floor[k],
                                "wide {wide} outside {outside}: copy {c} lost a write at {address:#x}: holds {value}, {} was written before the flush",
                                floor[k]
                            );
                            assert!(
                                value <= ceiling[w],
                                "wide {wide} outside {outside}: copy {c} holds {value} at {address:#x}, never written ({} at most)",
                                ceiling[w]
                            );
                            assert!(
                                value >= held[c][k],
                                "wide {wide} outside {outside}: copy {c} went back at {address:#x}: {value} after {}",
                                held[c][k]
                            );
                            held[c][k] = value;
                        }
                    }
                }
            }
            stop.store(true, Relaxed);
        });
        flush(&copies, wide);
        for (c, copy) in copies.iter().enumerate() {
            let start = copy.shadow.start as usize;
            let n = copy.side.len().min(at + len - start);
            // SAFETY: title memory we mapped; readable whether guarded or not.
            let now = unsafe { std::slice::from_raw_parts(start as *const u8, n) };
            assert!(
                copy.side[..n] == *now,
                "wide {wide} outside {outside}: copy {c} differs from title memory after a quiet flush"
            );
        }
    }
    crate::watch::GUARD_OUTSIDE_TEST.store(0, Relaxed);
}

/// Whether making blocks read-only scales with threads at all
/// (`cargo test -p victory-gpu --release protect_scaling -- --ignored
/// --nocapture`): each of 1, 2, 4 and 8 threads makes its own blocks of
/// title view memory read-only, a call a block, and the time all calls
/// took is said; `BENCH_SPIN` threads of the process spin meanwhile, as
/// the title's do (each change of protection is a flush of their TLBs).
#[test]
#[ignore = "benchmark: run by hand on a quiet machine"]
fn protect_scaling() {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    let blocks = env("BENCH_BLOCKS", 4096);
    let spinners = env("BENCH_SPIN", 4);
    let title = Title::new(blocks);
    let at = title.at;
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        let _stop = Stop(&stop);
        for _ in 0..spinners {
            let stop = &stop;
            s.spawn(move || {
                let mut x = 0u64;
                while !stop.load(Relaxed) {
                    x = x.wrapping_add(1);
                    std::hint::spin_loop();
                }
                x
            });
        }
        for threads in [1usize, 2, 4, 8] {
            let mut times = Vec::new();
            for _ in 0..9 {
                let started = std::time::Instant::now();
                std::thread::scope(|s| {
                    for t in 0..threads {
                        s.spawn(move || {
                            for block in (t..blocks).step_by(threads) {
                                let mut old = 0;
                                // SAFETY: our view memory; only its protection changes.
                                unsafe {
                                    // Its pages there and in use, as the title's are.
                                    for page in 0..BLOCK / 4096 {
                                        ((at + block * BLOCK + page * 4096) as *mut u8)
                                            .write_volatile(1);
                                    }
                                    VirtualProtect(
                                        (at + block * BLOCK) as *const _,
                                        BLOCK,
                                        PAGE_READONLY,
                                        &mut old,
                                    );
                                    VirtualProtect(
                                        (at + block * BLOCK) as *const _,
                                        BLOCK,
                                        PAGE_READWRITE,
                                        &mut old,
                                    );
                                }
                            }
                        });
                    }
                });
                times.push(started.elapsed().as_secs_f64() * 1e3);
            }
            let m = median(times);
            println!(
                "{threads} threads: {} blocks made read-only and writable again in {m:.2} ms ({:.2} us a call), {spinners} threads spinning",
                blocks,
                m * 1e3 / (2 * blocks) as f64
            );
        }
    });
}
