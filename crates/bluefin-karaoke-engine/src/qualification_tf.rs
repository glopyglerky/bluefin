//! Offline-only TensorFlow 2.21 C ABI reference runner. Never linked into the engine.
//! The library is loaded only from the explicit operator-owned qualification directory.
use super::{Graph, MetaGraph};
use crate::checkpoint::Checkpoint;
use prost::Message;
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
};
type P = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Output {
    oper: P,
    index: c_int,
}
#[derive(Clone, PartialEq, Message)]
struct Shape {
    #[prost(message, repeated, tag = "2")]
    dims: Vec<Dim>,
}
#[derive(Clone, PartialEq, Message)]
struct Dim {
    #[prost(int64, tag = "1")]
    size: i64,
}
#[derive(Clone, PartialEq, Message)]
struct TensorProto {
    #[prost(int32, tag = "1")]
    dtype: i32,
    #[prost(message, optional, tag = "2")]
    shape: Option<Shape>,
    #[prost(bytes, tag = "4")]
    content: Vec<u8>,
}
#[derive(Clone, PartialEq, Message)]
struct TensorAttr {
    #[prost(message, optional, tag = "8")]
    tensor: Option<TensorProto>,
}

fn frozen(c: &Checkpoint, outputs: &[&str]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut graph = MetaGraph::decode(c.meta.as_slice())?
        .graph
        .ok_or("missing graph")?;
    let original: HashMap<_, _> = graph
        .nodes
        .iter()
        .map(|n| (n.name.clone(), n.clone()))
        .collect();
    for n in &mut graph.nodes {
        n.device.clear();
        n.attrs.remove("_class");
        if n.op == "ReadVariableOp" {
            let mut name = n
                .inputs
                .first()
                .ok_or("missing variable")?
                .split(':')
                .next()
                .unwrap();
            for _ in 0..16 {
                if c.weights.contains_key(name) {
                    break;
                }
                let node = original.get(name).ok_or("missing resource node")?;
                if !["Switch", "Identity"].contains(&node.op.as_str()) {
                    return Err(format!("unsupported resource node {name}").into());
                }
                name = node
                    .inputs
                    .first()
                    .ok_or("missing resource input")?
                    .split(':')
                    .next()
                    .unwrap();
            }
            let w = c
                .weights
                .get(name)
                .ok_or_else(|| format!("missing weight {name}"))?;
            let value = TensorAttr {
                tensor: Some(TensorProto {
                    dtype: 1,
                    shape: Some(Shape {
                        dims: w.shape.iter().map(|d| Dim { size: *d as i64 }).collect(),
                    }),
                    content: w.values.iter().flat_map(|v| v.to_le_bytes()).collect(),
                }),
            }
            .encode_to_vec();
            n.op = "Const".into();
            n.inputs.clear();
            n.attrs = HashMap::from([("dtype".into(), vec![48, 1]), ("value".into(), value)]);
        }
    }
    let nodes: HashMap<_, _> = graph.nodes.iter().map(|n| (n.name.as_str(), n)).collect();
    let mut wanted = HashSet::new();
    let mut pending: Vec<String> = outputs.iter().map(|s| s.to_string()).collect();
    while let Some(name) = pending.pop() {
        let name = name.trim_start_matches('^').split(':').next().unwrap();
        if wanted.insert(name.to_owned()) {
            let node = nodes
                .get(name)
                .ok_or_else(|| format!("missing node {name}"))?;
            pending.extend(node.inputs.iter().cloned());
        }
    }
    graph.nodes.retain(|n| wanted.contains(&n.name));
    if graph
        .nodes
        .iter()
        .any(|n| n.op.contains("Variable") || n.op.contains("Assign") || n.op.contains("PyFunc"))
    {
        return Err("unfrozen or executable reference graph".into());
    }
    Ok(Graph { nodes: graph.nodes }.encode_to_vec())
}

pub fn run(
    root: &Path,
    checkpoint: &Checkpoint,
    input_name: &str,
    input: &[f32],
    dims: &[i64],
    outputs: &[&str],
) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
    if dims.iter().any(|d| *d <= 0)
        || dims
            .iter()
            .try_fold(1usize, |a, d| a.checked_mul(*d as usize))
            != Some(input.len())
        || input.iter().any(|x| !x.is_finite())
    {
        return Err("invalid reference input".into());
    }
    let graph_bytes = frozen(checkpoint, outputs)?;
    // SAFETY: all symbols below are the stable TF C ABI. The pinned publisher wheel
    // library outlives every call/allocation; buffers remain live through synchronous Run.
    unsafe {
        let library = libloading::Library::new(root.join("libtensorflow_cc.so.2"))?;
        macro_rules! api {
            ($name:literal, $ty:ty) => {
                *library.get::<$ty>(concat!($name, "\0").as_bytes())?
            };
        }
        let version = api!("TF_Version", unsafe extern "C" fn() -> *const c_char);
        println!(
            "reference TensorFlow={}",
            CStr::from_ptr(version()).to_string_lossy()
        );
        let new_status = api!("TF_NewStatus", unsafe extern "C" fn() -> P);
        let delete_status = api!("TF_DeleteStatus", unsafe extern "C" fn(P));
        let get_code = api!("TF_GetCode", unsafe extern "C" fn(P) -> c_int);
        let message = api!("TF_Message", unsafe extern "C" fn(P) -> *const c_char);
        let new_graph = api!("TF_NewGraph", unsafe extern "C" fn() -> P);
        let delete_graph = api!("TF_DeleteGraph", unsafe extern "C" fn(P));
        let new_buffer = api!(
            "TF_NewBufferFromString",
            unsafe extern "C" fn(*const c_void, usize) -> P
        );
        let delete_buffer = api!("TF_DeleteBuffer", unsafe extern "C" fn(P));
        let new_import = api!("TF_NewImportGraphDefOptions", unsafe extern "C" fn() -> P);
        let delete_import = api!("TF_DeleteImportGraphDefOptions", unsafe extern "C" fn(P));
        let import = api!("TF_GraphImportGraphDef", unsafe extern "C" fn(P, P, P, P));
        let new_options = api!("TF_NewSessionOptions", unsafe extern "C" fn() -> P);
        let delete_options = api!("TF_DeleteSessionOptions", unsafe extern "C" fn(P));
        let set_config = api!(
            "TF_SetConfig",
            unsafe extern "C" fn(P, *const c_void, usize, P)
        );
        let new_session = api!("TF_NewSession", unsafe extern "C" fn(P, P, P) -> P);
        let close_session = api!("TF_CloseSession", unsafe extern "C" fn(P, P));
        let delete_session = api!("TF_DeleteSession", unsafe extern "C" fn(P, P));
        let op = api!(
            "TF_GraphOperationByName",
            unsafe extern "C" fn(P, *const c_char) -> P
        );
        let allocate = api!(
            "TF_AllocateTensor",
            unsafe extern "C" fn(c_int, *const i64, c_int, usize) -> P
        );
        let tensor_data = api!("TF_TensorData", unsafe extern "C" fn(P) -> *mut c_void);
        let tensor_size = api!("TF_TensorByteSize", unsafe extern "C" fn(P) -> usize);
        let delete_tensor = api!("TF_DeleteTensor", unsafe extern "C" fn(P));
        let session_run = api!(
            "TF_SessionRun",
            unsafe extern "C" fn(
                P,
                P,
                *const Output,
                *const P,
                c_int,
                *const Output,
                *mut P,
                c_int,
                *const P,
                c_int,
                P,
                P,
            )
        );
        let status = new_status();
        let graph = new_graph();
        let buffer = new_buffer(graph_bytes.as_ptr().cast(), graph_bytes.len());
        let options = new_import();
        import(graph, buffer, options, status);
        delete_import(options);
        delete_buffer(buffer);
        let check = || -> Result<(), Box<dyn std::error::Error>> {
            if get_code(status) == 0 {
                Ok(())
            } else {
                Err(CStr::from_ptr(message(status))
                    .to_string_lossy()
                    .to_string()
                    .into())
            }
        };
        if let Err(e) = check() {
            delete_graph(graph);
            delete_status(status);
            return Err(e);
        }
        let options = new_options();
        // ConfigProto: intra_op_parallelism_threads=3, inter_op_parallelism_threads=1.
        let config = [0x10, 3, 0x28, 1];
        set_config(options, config.as_ptr().cast(), config.len(), status);
        let session = new_session(graph, options, status);
        delete_options(options);
        if let Err(e) = check() {
            delete_graph(graph);
            delete_status(status);
            return Err(e);
        }
        let result = (|| {
            let input_op = Output {
                oper: op(graph, CString::new(input_name)?.as_ptr()),
                index: 0,
            };
            if input_op.oper.is_null() {
                return Err("missing input operation".into());
            }
            let mut output_ops = Vec::new();
            for name in outputs {
                let out = op(graph, CString::new(*name)?.as_ptr());
                if out.is_null() {
                    return Err("missing output operation".into());
                }
                output_ops.push(Output {
                    oper: out,
                    index: 0,
                });
            }
            let tensor = allocate(1, dims.as_ptr(), dims.len() as c_int, input.len() * 4);
            if tensor.is_null() {
                return Err("reference allocation failed".into());
            }
            std::ptr::copy_nonoverlapping(
                input.as_ptr().cast::<u8>(),
                tensor_data(tensor).cast(),
                input.len() * 4,
            );
            let mut out = vec![std::ptr::null_mut(); outputs.len()];
            session_run(
                session,
                std::ptr::null_mut(),
                &input_op,
                &tensor,
                1,
                output_ops.as_ptr(),
                out.as_mut_ptr(),
                out.len() as c_int,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                status,
            );
            delete_tensor(tensor);
            let result = check().and_then(|_| {
                out.iter()
                    .map(|t| {
                        if t.is_null()
                            || tensor_size(*t) > 64 * 1024 * 1024
                            || tensor_size(*t) % 4 != 0
                        {
                            return Err("invalid reference output".into());
                        }
                        let values = std::slice::from_raw_parts(
                            tensor_data(*t).cast::<f32>(),
                            tensor_size(*t) / 4,
                        )
                        .to_vec();
                        if values.iter().any(|x| !x.is_finite()) {
                            return Err("non-finite reference output".into());
                        }
                        Ok(values)
                    })
                    .collect()
            });
            for t in out {
                if !t.is_null() {
                    delete_tensor(t);
                }
            }
            result
        })();
        close_session(session, status);
        delete_session(session, status);
        delete_graph(graph);
        delete_status(status);
        result
    }
}
