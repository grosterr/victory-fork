use std::time::Instant;
use victory_shader::emit::{self, HostShader};
use victory_shader::native::{ComputeIo, HwStage, NativeProgram, StageIo};
use victory_shader::{ResourceType, RootParam, RootType, UsedResource};
use windows::Win32::Graphics::{Direct3D12::*, Dxgi::*};

use super::tests::{check_debug, create, test_debug};
use super::{HeapDesc, Placement, SHADER_VISIBLE, VIEWS_KIND, create_cbv, heap, release};
use crate::draw::State;
use crate::host::{self, Host};
use crate::list::Detached;
use crate::object::{S_OK, This};

fn adapter(host: &Host) {
    // SAFETY: plain adapter queries.
    let adapter = unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
        let adapter: IDXGIAdapter1 = factory
            .EnumAdapterByLuid(host.device().GetAdapterLuid())
            .unwrap();
        adapter.GetDesc1().unwrap()
    };
    let end = adapter
        .Description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(adapter.Description.len());
    assert_eq!(adapter.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32, 0);
    eprintln!(
        "adapter: {}",
        String::from_utf16_lossy(&adapter.Description[..end])
    );
}

struct TestHeap(This);

impl TestHeap {
    fn new(count: usize) -> Self {
        let mut title = std::ptr::null();
        let desc = HeapDesc {
            kind: VIEWS_KIND,
            count: count as u32,
            flags: SHADER_VISIBLE,
            nodes: 0,
        };
        assert_eq!(
            create(
                desc,
                Placement {
                    address: 0,
                    size: 0
                },
                &mut title
            ),
            S_OK
        );
        Self(title)
    }

    fn cbv(&self, slot: u32, src: &[u32]) {
        let address = src.as_ptr() as u64;
        create_cbv(
            std::ptr::null(),
            &[
                address as u32,
                (address >> 32) as u32,
                (src.len() * 4) as u32,
                0,
            ],
            heap(self.0).handle(slot).unwrap(),
        );
    }
}

impl Drop for TestHeap {
    fn drop(&mut self) {
        release(self.0);
    }
}

fn used(kind: ResourceType, offset: u32, slot: u8) -> UsedResource {
    UsedResource {
        kind,
        root: RootType::Table,
        table_offset_dwords: offset * 8,
        lower_bound: 0,
        stride: 4,
        dwords: 4,
        user_data: 4,
        slot,
    }
}

fn kernel(used: &[UsedResource]) -> HostShader {
    // Scalar and vector CBV loads from AvatarManz's 3920dc2 test fixture.
    let mut words: Vec<u32> = vec![
        0xBE85_0380,
        0xF408_0202,
        0xFA00_0000,
        0xF408_0302,
        0xFA00_0020,
        0xF428_0404,
        0xFA00_0010,
        0xF420_0504,
        0x0000_0000,
    ];
    for (out, sgpr) in (16..21u32).enumerate() {
        words.push(0x7E00_0000 | 1 << 17 | 1 << 9 | sgpr);
        words.push(0x38 << 26 | 0x1C << 18 | 4 * out as u32);
        words.push(0x80 << 24 | 3 << 16 | 1 << 8);
    }
    words.push(0xBF81_0000);
    let native = NativeProgram {
        stage: HwStage::Cs,
        wave64: false,
        vgprs: 4,
        used: used.to_vec(),
        layout: vec![
            RootParam {
                root: RootType::Constants,
                table_offset_dwords: 0,
                sgprs: 4,
                base_register: 0,
                user_data: 0,
                slot: 0,
            },
            RootParam {
                root: RootType::Table,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 4,
                slot: 1,
            },
        ],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1,
            threads: [1, 1, 1],
        }),
        code: words.into_iter().flat_map(u32::to_le_bytes).collect(),
    };
    let p = victory_shader::spill::build(&native).unwrap();
    let st = victory_shader::structure::structure(&p).unwrap();
    let io = victory_shader::io::raise(&p, &native).unwrap();
    let res = victory_shader::resource::track(&p, &native.layout, native.stage, Some(&st)).unwrap();
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(b"DXBC");
    bytes[20] = 1;
    bytes[24] = 32;
    let c = victory_shader::dxbc::Container::parse(&bytes).unwrap();
    let mut shader = emit::emit(&c, &native, &p, &st, &io, &res).unwrap();
    emit::validator::Validator::load(emit::validator::Validator::default_path().as_deref())
        .expect("dxil.dll")
        .validate(&mut shader.dxil)
        .unwrap();
    eprintln!("DXIL validated: {} bytes", shader.dxil.len());
    shader
}

#[repr(align(256))]
struct Constants([u32; 68]);

fn mapped(resource: &ID3D12Resource, count: usize) -> Vec<u32> {
    let mut data = std::ptr::null_mut();
    // SAFETY: the caller waited for the copy of this many dwords.
    unsafe {
        resource.Map(0, None, Some(&mut data)).unwrap();
        let result = std::slice::from_raw_parts(data.cast::<u32>(), count).to_vec();
        resource.Unmap(0, None);
        result
    }
}

#[test]
#[ignore = "physical shader-model 6.6 GPU, dxil.dll and VICTORY_LATE_WAIT=3"]
fn bench_cbv_snapshot_bindings() {
    assert!(host::late());
    let device = host::host().expect("D3D12");
    adapter(device);
    let debug = test_debug(device);
    let checking = debug.is_some();
    super::host::probe_view_limit();
    let count = if checking { 128 } else { 22_000 };
    let rounds = if checking { 2 } else { 8 };
    let reuse = std::env::var_os("VICTORY_CBV_SNAPSHOT_REUSE").is_some_and(|v| v == "1");
    let used = [used(ResourceType::Cbv, 0, 1), used(ResourceType::Uav, 1, 1)];
    let shader = kernel(&used);
    let (root, _) = crate::pipeline::host::root_signature(
        device.device(),
        &[D3D12_ROOT_PARAMETER1 {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
            Anonymous: D3D12_ROOT_PARAMETER1_0 {
                Constants: D3D12_ROOT_CONSTANTS {
                    ShaderRegister: 0,
                    RegisterSpace: emit::USER_DATA_SPACE,
                    Num32BitValues: shader.user_data_dwords(),
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        }],
        &[],
        true,
    )
    .unwrap();
    // SAFETY: shader bytecode and the borrowed root signature remain live for the call.
    let pso: ID3D12PipelineState = unsafe {
        device
            .device()
            .CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::transmute_copy(&root),
                CS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: shader.dxil.as_ptr().cast(),
                    BytecodeLength: shader.dxil.len(),
                },
                ..Default::default()
            })
    }
    .unwrap();
    let cases: &[usize] = if checking {
        &[64]
    } else {
        &[64, 1024, 4096, 22000]
    };
    for &distinct in cases {
        let title = TestHeap::new(count * 2);
        let views = heap(title.0).host().unwrap();
        let samplers = super::host::HostHeap::new(true, 1).unwrap();

        let mut sources: Vec<_> = (0..distinct).map(|_| Constants([0; 68])).collect();
        for i in 0..count {
            let src = &sources[i % distinct].0[4..];
            assert_eq!(src.as_ptr() as usize % 256, 16);
            title.cbv(i as u32 * 2, src);
        }
        let bytes = count as u64 * 32;
        let output = device.uav_buffer(bytes).unwrap();
        let readback = device.read_back_buffer(bytes).unwrap();
        let times = device.read_back_buffer(16).unwrap();
        // SAFETY: plain creation of queue timestamps.
        let mut query: Option<ID3D12QueryHeap> = None;
        unsafe {
            device.device().CreateQueryHeap(
                &D3D12_QUERY_HEAP_DESC {
                    Type: D3D12_QUERY_HEAP_TYPE_TIMESTAMP,
                    Count: 2,
                    NodeMask: 0,
                },
                &mut query,
            )
        }
        .unwrap();
        let query = query.unwrap();
        let frequency = device.timestamp_frequency() as f64;
        assert!(frequency > 0.0);

        for round in 0..rounds {
            for (source, words) in sources.iter_mut().enumerate() {
                for (i, word) in words.0[4..].iter_mut().enumerate() {
                    *word = 0x50a0_0000 | ((round * distinct + source) as u32) << 8 | i as u32;
                }
            }
            let list = device.new_list().unwrap();
            let mut state = State::default();
            state.set_heaps(&[title.0]);
            // SAFETY: an open list and live objects retained through its fence.
            unsafe {
                list.list
                    .SetDescriptorHeaps(&[Some(views.heap.clone()), Some(samplers.heap.clone())]);
                list.list.SetComputeRootSignature(&root);
                list.list.SetPipelineState(&pso);
                list.list.EndQuery(&query, D3D12_QUERY_TYPE_TIMESTAMP, 0);
            }
            let change = checking && round == 1;
            let write_at = count / 2;
            let mut values = vec![0; shader.user_data_dwords() as usize];
            let started = Instant::now();
            for i in 0..count {
                if change && i == write_at {
                    let address = (&mut sources[0].0[8] as *mut u32) as usize;
                    list.defer_at(address, 4, move || {
                        // SAFETY: the test source stays live through submission.
                        unsafe { (address as *mut u32).write(0xaabb_ccdd) };
                    });
                }

                state.set_table(true, 1, heap(title.0).handle(i as u32 * 2).unwrap());
                let made = state.tables(&list, &used, &[], &[], true, &[], &mut Vec::new());
                let [(1, offset)] = made[..] else {
                    panic!("unexpected table {made:?}")
                };
                assert_ne!(offset, u32::MAX);
                views.raw_uav_at(offset / 32 + 1, &output, i as u64 * 32, 32);
                let scalar = [0, 252, 256][i % 3];
                for arg in &shader.user_data {
                    values[arg.index as usize] = match (arg.root, arg.slot, arg.dword) {
                        (RootType::Table, 1, _) => offset,
                        (RootType::Constants, 0, 0) => scalar,
                        _ => 0,
                    };
                }
                // SAFETY: all values and descriptors are live. Outputs are disjoint.
                unsafe {
                    list.list.SetComputeRoot32BitConstants(
                        0,
                        values.len() as u32,
                        values.as_ptr().cast(),
                        0,
                    );
                    list.list.Dispatch(1, 1, 1);
                }
            }
            let record = started.elapsed();
            let (fills, copied, chunks) = list.constant_bench_allocations();
            assert_eq!(fills, if reuse && !change { distinct } else { count });
            assert_eq!(copied, fills * 256);
            // SAFETY: the timestamp interval ends before transitions and readbacks.
            unsafe {
                list.list.EndQuery(&query, D3D12_QUERY_TYPE_TIMESTAMP, 1);
                list.list
                    .ResolveQueryData(&query, D3D12_QUERY_TYPE_TIMESTAMP, 0, 2, &times, 0);
                list.list.ResourceBarrier(&[host::transition(
                    &output,
                    D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                )]);
                list.list.CopyBufferRegion(&readback, 0, &output, 0, bytes);
            }
            list.close();
            // SAFETY: a test gate isolates submit preparation from GPU execution.
            let gate: ID3D12Fence =
                unsafe { device.device().CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();
            unsafe { device.queue().Wait(&gate, 1) }.unwrap();
            let started = Instant::now();
            device.execute_late(vec![Detached::with_state(list, state)], vec![0]);
            let submit = started.elapsed();
            if checking {
                // A CPU write after submission must not affect this flight.
                sources[0].0[4] ^= 0xffff;
            }
            // SAFETY: the queue can now consume the completed upload snapshots.
            unsafe { gate.Signal(1) }.unwrap();
            host::settle_for_fence();
            let ticks = mapped(&times, 4);
            let begin = u64::from(ticks[0]) | u64::from(ticks[1]) << 32;
            let end = u64::from(ticks[2]) | u64::from(ticks[3]) << 32;
            assert!(end > begin);
            let got = mapped(&readback, count * 8);
            for i in 0..count {
                let base = 0x50a0_0000 | ((round * distinct + i % distinct) as u32) << 8;
                let scalar = [base, base | 63, 0][i % 3];
                let vector_x = if change && i >= write_at && i % distinct == 0 {
                    0xaabb_ccdd
                } else {
                    base | 4
                };
                assert_eq!(
                    got[i * 8..i * 8 + 5],
                    [vector_x, base | 5, base | 6, base | 7, scalar],
                    "dispatch {i}"
                );
            }
            eprintln!(
                "reuse={reuse} distinct={distinct} round={round} bindings={count} record_ms={:.3} prepare_enqueue_ms={:.3} gpu_ms={:.3} fills={fills} bytes={copied} scratch_chunks={chunks}",
                record.as_secs_f64() * 1000.0,
                submit.as_secs_f64() * 1000.0,
                (end - begin) as f64 / frequency * 1000.0
            );
        }
    }
    check_debug(debug);
}

#[test]
fn a_cut_records_a_new_snapshot_even_with_the_same_table() {
    let Some(device) = host::host() else { return };
    let title = TestHeap::new(1);
    let data = Constants([0; 68]);
    title.cbv(0, &data.0[4..]);
    let layout = [used(ResourceType::Cbv, 0, 1)];
    let mut state = State::default();
    state.set_heaps(&[title.0]);
    state.set_table(true, 1, heap(title.0).handle(0).unwrap());
    let before = device.new_list().unwrap();
    let first = state.tables(&before, &layout, &[], &[], true, &[], &mut Vec::new());
    let after = device.new_list().unwrap();
    state.replay(&after);
    let next = state.tables(&after, &layout, &[], &[], true, &[], &mut Vec::new());
    assert_ne!(first, next);
    assert_eq!(before.constant_bench_allocations().0, 1);
    assert_eq!(after.constant_bench_allocations().0, 1);
    after.defer_at(data.0.as_ptr() as usize, 256, || {});
    let post_write = state.tables(&after, &layout, &[], &[], true, &[], &mut Vec::new());
    assert_ne!(next, post_write);
    assert_eq!(after.constant_bench_allocations().0, 2);
}

#[test]
fn constant_snapshots_observe_copy_epochs_and_fall_back_for_partial_views() {
    let Some(device) = host::host() else { return };
    let list = device.new_list().unwrap();
    let source = Constants([0; 68]);
    let pointer = source.0[4..].as_ptr() as u64;
    let first = list.constant_scratch(pointer, 256).unwrap();
    let same = list.constant_scratch(pointer, 256).unwrap();
    let reuse = std::env::var_os("VICTORY_CBV_SNAPSHOT_REUSE").is_some_and(|v| v == "1");
    assert_eq!(first == same, reuse);
    let mut dest = 0u8;
    let from = 1u8;
    list.defer_copy(&mut dest, &from, 1);
    let next = list.constant_scratch(pointer, 256).unwrap();
    assert_ne!(first, next);
    let partial = list.constant_scratch(pointer, 240).unwrap();
    assert_ne!(partial, list.constant_scratch(pointer, 240).unwrap());
}
