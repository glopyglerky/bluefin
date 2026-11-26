//! Offline qualification only. Not enabled by the server dependency.
use crate::{Cancellation, checkpoint::Checkpoint};
use prost::Message;
use std::collections::HashMap;
#[path = "qualification_fixture.rs"]
mod fixture;
#[path = "qualification_tf.rs"]
mod tf;
#[derive(Clone, PartialEq, Message)]
pub struct MetaGraph {
    #[prost(message, optional, tag = "2")]
    pub graph: Option<Graph>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Graph {
    #[prost(message, repeated, tag = "1")]
    pub nodes: Vec<Node>,
}
#[derive(Clone, PartialEq, Message)]
pub struct Node {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub op: String,
    #[prost(string, repeated, tag = "3")]
    pub inputs: Vec<String>,
    #[prost(string, tag = "4")]
    pub device: String,
    #[prost(map = "string, bytes", tag = "5")]
    pub attrs: HashMap<String, Vec<u8>>,
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("inspect") => {
            let checkpoint = Checkpoint::from_official_archive(&std::fs::read(&args[2])?, &Cancellation::default())?;
            for (name, hash) in &checkpoint.member_hashes { println!("{hash}  {name}"); }
            for (name, w) in &checkpoint.weights { println!("{name} {:?}", w.shape); }
            std::fs::write(&args[3], &checkpoint.meta)?;
            let graph = MetaGraph::decode(checkpoint.meta.as_slice())?.graph.ok_or("no graph")?;
            for n in &graph.nodes {
                if !n.name.contains("Adam") && !n.name.contains("gradients") && !n.name.contains("Initializer") {
                    println!("NODE {} {} {:?} {:?}", n.name, n.op, n.inputs, n.attrs);
                }
            }
        }
        Some("fetch-reference") => {
            let root = std::path::Path::new(&args[2]);
            std::fs::create_dir_all(root)?;
            let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(600)).build()?;
            let metadata = client.get("https://pypi.org/pypi/tensorflow/2.21.0/json").send()?.error_for_status()?.bytes()?;
            std::fs::write(root.join("tensorflow-2.21.0-pypi.json"), &metadata)?;
            let metadata: serde_json::Value = serde_json::from_slice(&metadata)?;
            let file = metadata["urls"].as_array().ok_or("no urls")?.iter().find(|v| v["filename"] == "tensorflow-2.21.0-cp313-cp313-manylinux_2_27_x86_64.whl").ok_or("no pinned platform wheel")?;
            let url = file["url"].as_str().ok_or("no url")?;
            if !url.starts_with("https://files.pythonhosted.org/") { return Err("unexpected publisher endpoint".into()); }
            let hash = file["digests"]["sha256"].as_str().ok_or("no hash")?;
            let path = root.join("tensorflow.whl");
            if !path.exists() {
                let mut file = std::fs::File::create_new(&path)?;
                std::io::copy(&mut client.get(url).send()?.error_for_status()?, &mut file)?;
            }
            let bytes = std::fs::read(&path)?;
            if crate::checkpoint::digest(&bytes) != hash { return Err("wheel hash mismatch".into()); }
            println!("reference wheel sha256={hash}");
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
            for name in ["tensorflow/libtensorflow_cc.so.2", "tensorflow/libtensorflow_framework.so.2"] {
                let mut member = zip.by_name(name)?;
                let mut file = std::fs::File::create(root.join(name.rsplit('/').next().unwrap()))?;
                std::io::copy(&mut member, &mut file)?;
            }
        }
        Some("parity") => {
            let c = Checkpoint::from_official_archive(&std::fs::read(&args[2])?, &Cancellation::default())?;
            let model = crate::model::Model::load(&c)?;
            let input: Vec<f32> = (0..2*512*1024).map(|i| (((i*17+113)%1009) as f32 / 1009.).powi(3)*10.).collect();
            let start = std::time::Instant::now();
            let actual = model.infer(&input,&Cancellation::default())?;
            println!("Rust inference seconds={}",start.elapsed().as_secs_f64());
            let nhwc: Vec<f32> = (0..512*1024).flat_map(|i| [input[i],input[512*1024+i]]).collect();
            let expected = tf::run(std::path::Path::new(&args[3]),&c,"strided_slice_3",&nhwc,&[1,512,1024,2],&["vocals_spectrogram/mul","accompaniment_spectrogram/mul"])?;
            for stem in 0..2 {
                let mut max = 0f32;
                let mut sq = 0f64;
                for (i,value) in actual[stem].iter().enumerate() {
                    let j = (i%(512*1024))*2+i/(512*1024);
                    let diff = value-expected[stem][j];
                    max = max.max(diff.abs()); sq += f64::from(diff).powi(2);
                }
                let rms = (sq/input.len() as f64).sqrt();
                println!("stem={stem} max_abs={max} rms={rms}");
                if max > 0.002 || rms > 0.0001 { return Err("TensorFlow parity failed".into()); }
            }
        }
        Some("audio") => {
            use crate::{Engine,Config,Input,StagingRoot,Stem,Cancellation};
            use sha2::{Digest,Sha256};
            use std::{io::Read,os::unix::fs::PermissionsExt};
            let c=Cancellation::default();
            let archive=std::fs::read(&args[2])?;
            let checkpoint=Checkpoint::from_official_archive(&archive,&c)?;
            let mut engine=Engine::load_official_archive(&archive,Config::default(),&c)?;
            let root=std::path::Path::new(&args[4]); std::fs::create_dir_all(root)?;
            let task_root=root.join("task-staging"); std::fs::create_dir_all(&task_root)?;
            std::fs::set_permissions(&task_root,std::fs::Permissions::from_mode(0o700))?;
            let staging=StagingRoot::from_opened_directory(std::fs::File::open(&task_root)?)?;
            let [mix,vocal,backing]=fixture::synth(13.);
            let wav=crate::audio::encode(&mix,&c)?;
            let actual_mix=crate::audio::samples(&wav);
            let processed=engine.process(Input { pcm_wav:&wav,expected_pcm_sha256:Sha256::digest(&wav).into(),attachment:"opaque-parent-attachment" },&staging,&c)?;
            if processed.attachment!="opaque-parent-attachment" { return Err("attachment lost".into()); }
            let mut outputs=Vec::new();
            std::fs::write(root.join("original-synthetic-mixture.wav"),&wav)?;
            std::fs::write(root.join("original-synthetic-vocal.wav"),crate::audio::encode(&vocal,&c)?)?;
            std::fs::write(root.join("original-synthetic-backing.wav"),crate::audio::encode(&backing,&c)?)?;
            for (stem,name) in [(Stem::LeadVocal,"lead-vocal.wav"),(Stem::Accompaniment,"accompaniment.wav")] {
                let mut bytes=Vec::new(); processed.files.open_stem(stem)?.read_to_end(&mut bytes)?;
                if crate::audio::validate(&bytes,300)?!=mix.len()/2 { return Err("output duration mismatch".into()); }
                std::fs::write(root.join(name),&bytes)?; outputs.push(crate::audio::samples(&bytes));
            }
            std::fs::write(root.join("provenance.json"),serde_json::to_vec_pretty(&processed.provenance)?)?;
            let reference=tf::run(std::path::Path::new(&args[3]),&checkpoint,"waveform",&actual_mix,&[(mix.len()/2) as i64,2],&["strided_slice_13","strided_slice_23"])?;
            let mut parity=Vec::new();
            for stem in 0..2 {
                if reference[stem].len()!=outputs[stem].len() { return Err("reference duration mismatch".into()); }
                let max=reference[stem].iter().zip(&outputs[stem]).map(|(a,b)|(a-b).abs()).fold(0f32,f32::max);
                let snr=fixture::snr(&reference[stem],&outputs[stem],0);
                println!("audio stem={stem} reference max_abs={max} snr_db={snr}");
                if max>0.002 || snr<55. { return Err("end-to-end waveform parity failed".into()); }
                parity.push(serde_json::json!({"stem":stem,"maxAbs":max,"snrDb":snr}));
            }
            let reconstructed:Vec<f32>=outputs[0].iter().zip(&outputs[1]).map(|(a,b)|a+b).collect();
            let reconstruction=fixture::snr(&actual_mix,&reconstructed,crate::audio::RATE/5*2);
            let quality=serde_json::json!({
                "fixture":"original additive synthesized vowel + plucked chords/bass/filtered percussion; 13 s stereo",
                "inputWavSha256":crate::checkpoint::digest(&wav),"waveformParity":parity,
                "interiorReconstructionSnrDb":reconstruction,"excludedLeadingSeconds":0.2,
                "vocalSiSdrDb":fixture::si_sdr(&vocal,&outputs[0]),"inputVocalSiSdrDb":fixture::si_sdr(&vocal,&actual_mix),
                "backingSiSdrDb":fixture::si_sdr(&backing,&outputs[1]),"inputBackingSiSdrDb":fixture::si_sdr(&backing,&actual_mix),
                "inferenceAndPublicationSeconds":processed.provenance.elapsed_seconds,
                "gpu":"unavailable; CPU only; not qualified on GPU"
            });
            println!("{}",serde_json::to_string_pretty(&quality)?);
            std::fs::write(root.join("qualification.json"),serde_json::to_vec_pretty(&quality)?)?;
            processed.files.remove()?;
            if reconstruction<20. || fixture::si_sdr(&vocal,&outputs[0])<fixture::si_sdr(&vocal,&actual_mix)+2.
                || fixture::si_sdr(&backing,&outputs[1])<fixture::si_sdr(&backing,&actual_mix)+6.
                || std::fs::read_dir(&task_root)?.count()!=0 { return Err("quality/reconstruction/cleanup sanity failed".into()); }
        }
        Some("bench") => {
            use crate::{Engine,Config,Input,StagingRoot,Cancellation};
            use sha2::{Digest,Sha256};
            use std::os::unix::fs::PermissionsExt;
            let c=Cancellation::default();
            let archive=std::fs::read(&args[2])?;
            let initial=std::time::Instant::now();
            let mut engine=Engine::load_official_archive(&archive,Config::default(),&c)?;
            let initialize=initial.elapsed().as_secs_f64(); drop(archive);
            let seconds:usize=args[4].parse()?;
            if !(1..=300).contains(&seconds) {return Err("bench duration outside bounds".into());}
            let samples:Vec<f32>=(0..crate::audio::RATE*seconds*2).map(|i| {
                let t=(i/2) as f32/crate::audio::RATE as f32;
                (std::f32::consts::TAU*220.*t).sin()*0.1+(std::f32::consts::TAU*329.63*t).sin()*0.05
            }).collect();
            let wav=crate::audio::encode(&samples,&c)?; drop(samples);
            let root=std::path::Path::new(&args[3]);std::fs::create_dir_all(root)?;
            let task_root=root.join("bench-staging");std::fs::create_dir_all(&task_root)?;
            std::fs::set_permissions(&task_root,std::fs::Permissions::from_mode(0o700))?;
            let staging=StagingRoot::from_opened_directory(std::fs::File::open(&task_root)?)?;
            let start=std::time::Instant::now();
            let output=engine.process(Input { pcm_wav:&wav,expected_pcm_sha256:Sha256::digest(&wav).into(),attachment:() },&staging,&c)?;
            let elapsed=start.elapsed().as_secs_f64();
            let status=std::fs::read_to_string("/proc/self/status")?;
            let memory:Vec<_>=status.lines().filter(|l|l.starts_with("VmHWM:")||l.starts_with("VmRSS:")||l.starts_with("Threads:")).collect();
            let report=serde_json::json!({"audioSeconds":seconds,"initializeSeconds":initialize,"processSeconds":elapsed,"cpuThreads":3,"processMemory":memory,"provenance":output.provenance,"referenceRuntimeLoaded":false});
            output.files.remove()?;
            if std::fs::read_dir(&task_root)?.count()!=0 {return Err("bench staging not empty".into());}
            println!("{}",serde_json::to_string_pretty(&report)?);
            std::fs::write(root.join(format!("benchmark-{seconds}s.json")),serde_json::to_vec_pretty(&report)?)?;
        }
        Some("checkpoint") => {
            let source=std::fs::canonicalize(&args[2])?;
            let output=std::path::Path::new(&args[3]);
            let mut paths=Vec::new();
            collect_source(&source.join("crates/bluefin-karaoke-engine"),&mut paths)?;
            paths.push(source.join("docs/bluefin/karaoke-engine.md")); paths.sort();
            let mut archive=tar::Builder::new(std::fs::File::create_new(output)?);
            let mut manifest=std::collections::BTreeMap::new();
            for path in paths {
                let name=path.strip_prefix(&source)?.to_string_lossy().to_string();
                let bytes=std::fs::read(&path)?;
                let mut header=tar::Header::new_gnu();header.set_mode(0o644);header.set_size(bytes.len() as u64);header.set_mtime(0);header.set_cksum();
                archive.append_data(&mut header,&name,bytes.as_slice())?;
                manifest.insert(name,crate::checkpoint::digest(&bytes));
            }
            archive.finish()?; drop(archive);
            manifest.insert("SOURCE_ARCHIVE_SHA256".into(),crate::checkpoint::digest(&std::fs::read(output)?));
            std::fs::write(output.with_extension("manifest.json"),serde_json::to_vec_pretty(&manifest)?)?;
        }
        _ => return Err("usage: qualify inspect ARCHIVE META_OUTPUT | fetch-reference DIR | parity ARCHIVE REFERENCE_DIR | audio ARCHIVE REFERENCE_DIR OUTPUT_DIR | bench ARCHIVE OUTPUT_DIR SECONDS | checkpoint SOURCE_ROOT OUTPUT_TAR".into()),
    }
    Ok(())
}

fn collect_source(
    root: &std::path::Path,
    output: &mut Vec<std::path::PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err("source checkpoint refuses symlinks".into());
        }
        if kind.is_dir() {
            if ["target", ".git"].contains(&entry.file_name().to_string_lossy().as_ref()) {
                return Err("unexpected generated/history directory inside owned crate".into());
            }
            collect_source(&entry.path(), output)?;
        } else if kind.is_file() {
            output.push(entry.path());
        } else {
            return Err("unexpected source file type".into());
        }
    }
    Ok(())
}
