//! FFmpeg jobs are bounded and stopped when their playback stops being requested.
use std::{collections::HashMap, path::{Path,PathBuf}, sync::LazyLock, time::{Duration,Instant}};
use tokio::{process::{Child,Command},sync::Mutex};
use serde::Deserialize;

struct Job { child: Child, directory:PathBuf, touched:Instant }
static JOBS: LazyLock<Mutex<HashMap<String,Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug,Clone)]
pub struct SourceInfo {pub width:u32,pub height:u32,pub bitrate_kbps:u32,pub duration:f64}
#[derive(Deserialize)]
struct Probe {streams:Vec<Stream>, format:Format}
#[derive(Deserialize)]
struct Stream {codec_type:Option<String>,width:Option<u32>,height:Option<u32>}
#[derive(Deserialize)]
struct Format {duration:Option<String>,bit_rate:Option<String>}

fn command(name:&str,key:&str)->Command{
    let mut command=Command::new(std::env::var_os(key).unwrap_or_else(||name.into()));
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}
pub async fn probe(path:&Path)->Result<SourceInfo,String>{
    let mut cmd=command("ffprobe","LISEK_FFPROBE");
    cmd.args(["-v","error","-show_streams","-show_format","-of","json"]).arg(path);
    let output=tokio::time::timeout(Duration::from_secs(20),cmd.output()).await.map_err(|_|"probe_timed_out")?.map_err(|_|"ffprobe_unavailable")?;
    if !output.status.success(){return Err("probe_failed".into());}
    let probe:Probe=serde_json::from_slice(&output.stdout).map_err(|_|"probe_failed")?;
    let video=probe.streams.iter().find(|s|s.codec_type.as_deref()==Some("video")).ok_or("video_stream_missing")?;
    let duration=probe.format.duration.and_then(|v|v.parse::<f64>().ok()).filter(|v|v.is_finite()&&*v>0.0).ok_or("duration_unavailable")?;
    Ok(SourceInfo{width:video.width.ok_or("resolution_unavailable")?,height:video.height.ok_or("resolution_unavailable")?,
        bitrate_kbps:probe.format.bit_rate.and_then(|v|v.parse::<u64>().ok()).map(|v|(v/1000).min(u32::MAX as u64)as u32).unwrap_or(0),duration})
}
pub async fn start(id:&str,path:&Path,directory:&Path,offset:f64)->Result<(),String>{
    let mut jobs=JOBS.lock().await;
    let active=jobs.values_mut().map(|job| usize::from(job.child.try_wait().ok().flatten().is_none())).sum::<usize>();
    if active>=2{return Err("transcode_capacity_reached".into());}
    tokio::fs::create_dir_all(directory).await.map_err(|_|"transcode_directory_unavailable")?;
    let mut cmd=command("ffmpeg","LISEK_FFMPEG");
    cmd.args(["-hide_banner","-loglevel","error","-nostdin","-y","-ss",&offset.to_string(),"-re","-i"]).arg(path)
        .args(["-map","0:v:0","-map","0:a:0?","-sn","-c:v","libx264","-preset","veryfast","-pix_fmt","yuv420p",
               "-vf","scale=w=1280:h=720:force_original_aspect_ratio=decrease:force_divisible_by=2",
               "-b:v","2500k","-maxrate","3000k","-bufsize","6000k","-c:a","aac","-b:a","128k",
               "-force_key_frames","expr:gte(t,n_forced*4)","-f","hls","-hls_time","4","-hls_list_size","0",
               "-hls_playlist_type","event","-hls_flags","temp_file","-hls_segment_filename"])
        .arg(directory.join("segment-%06d.ts")).arg(directory.join("index.m3u8"))
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let child=cmd.spawn().map_err(|_|"ffmpeg_unavailable")?;
    jobs.insert(id.to_owned(),Job{child,directory:directory.to_owned(),touched:Instant::now()});
    Ok(())
}
pub async fn touch(id:&str)->bool{
    if let Some(job)=JOBS.lock().await.get_mut(id){job.touched=Instant::now();true}else{false}
}
pub async fn exists(id:&str)->bool{JOBS.lock().await.contains_key(id)}
pub async fn stop(id:&str){
    if let Some(mut job)=JOBS.lock().await.remove(id){
        let _=job.child.kill().await;
        let _=job.child.wait().await;
        let _=tokio::fs::remove_dir_all(&job.directory).await;
    }
}
pub async fn cleanup(){
    let ids:Vec<String>=JOBS.lock().await.iter().filter(|(_,job)|job.touched.elapsed()>Duration::from_secs(90)).map(|(id,_)|id.clone()).collect();
    for id in ids{stop(&id).await;}
}
pub fn valid_file(name:&str)->bool{
    name=="index.m3u8" || name.strip_prefix("segment-").and_then(|s|s.strip_suffix(".ts")).is_some_and(|n|n.len()==6&&n.bytes().all(|b|b.is_ascii_digit()))
}
#[cfg(test)]
mod tests{
    #[test]
    fn only_allows_generated_media_filenames(){
        for name in ["index.m3u8","segment-000012.ts"]{assert!(super::valid_file(name));}
        for name in ["../index.m3u8","segment-../../secret.ts","anything.ts","segment-1.ts","/index.m3u8"]{assert!(!super::valid_file(name));}
    }
}
