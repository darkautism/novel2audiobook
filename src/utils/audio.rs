use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Merges multiple audio files via simple binary concatenation.
/// Suitable for MP3 or other stream-based formats.
pub fn merge_binary_files(input_paths: &[std::path::PathBuf], output_path: &Path) -> Result<()> {
    if input_paths.is_empty() {
        return Ok(());
    }
    let mut final_file = File::create(output_path)?;
    for path in input_paths {
        let mut f = File::open(path)?;
        std::io::copy(&mut f, &mut final_file)?;
    }
    Ok(())
}

struct WavInfo {
    fmt_content: Vec<u8>,
    data_offset: u64,
    data_size: u32,
}

fn scan_wav(path: &Path) -> Result<WavInfo> {
    let mut f = File::open(path)?;
    
    // Check RIFF
    let mut id = [0u8; 4];
    f.read_exact(&mut id)?;
    if &id != b"RIFF" { return Err(anyhow!("Not a RIFF file")); }
    
    // Skip File Size
    f.seek(SeekFrom::Current(4))?;
    
    // Check WAVE
    f.read_exact(&mut id)?;
    if &id != b"WAVE" { return Err(anyhow!("Not a WAVE file")); }
    
    let mut fmt_content: Option<Vec<u8>> = None;
    let mut data_offset: Option<u64> = None;
    let mut data_size: Option<u32> = None;
    
    loop {
        let mut chunk_id = [0u8; 4];
        let n = f.read(&mut chunk_id)?;
        if n == 0 { break; } // EOF
        if n < 4 { return Err(anyhow!("Unexpected EOF reading chunk ID")); }
        
        let mut size_buf = [0u8; 4];
        f.read_exact(&mut size_buf)?;
        let chunk_size = u32::from_le_bytes(size_buf);
        
        if &chunk_id == b"fmt " {
            let mut buf = vec![0u8; chunk_size as usize];
            f.read_exact(&mut buf)?;
            fmt_content = Some(buf);
        } else if &chunk_id == b"data" {
            data_offset = Some(f.stream_position()?);
            data_size = Some(chunk_size);
            // We found data, stop scanning
            break; 
        } else {
            // Skip unknown chunk
            f.seek(SeekFrom::Current(chunk_size as i64))?;
        }
    }
    
    Ok(WavInfo {
        fmt_content: fmt_content.ok_or_else(|| anyhow!("Missing fmt chunk in {:?}", path))?,
        data_offset: data_offset.ok_or_else(|| anyhow!("Missing data chunk in {:?}", path))?,
        data_size: data_size.ok_or_else(|| anyhow!("Missing data chunk size in {:?}", path))?,
    })
}

/// Merges multiple WAV files by parsing headers and concatenating data chunks.
/// Ensures all files have compatible format (fmt chunk).
pub fn merge_wav_files(input_paths: &[std::path::PathBuf], output_path: &Path) -> Result<()> {
    if input_paths.is_empty() {
        return Ok(());
    }

    // 1. Scan all files to gather info and calculate total size
    let mut total_data_size: u32 = 0;
    let mut infos = Vec::with_capacity(input_paths.len());
    
    let first_info = scan_wav(&input_paths[0])?;
    let base_fmt = first_info.fmt_content.clone();
    
    total_data_size += first_info.data_size;
    infos.push(first_info);
    
    for path in &input_paths[1..] {
        let info = scan_wav(path).with_context(|| format!("Failed to parse WAV {:?}", path))?;
        
        // Verify format compatibility
        if info.fmt_content != base_fmt {
            return Err(anyhow!("WAV format mismatch in file {:?}. All segments must have same sample rate/channels.", path));
        }
        
        total_data_size += info.data_size;
        infos.push(info);
    }
    
    // 2. Write Output
    let mut out = File::create(output_path)?;
    
    // Construct Header
    // RIFF [4] + Size [4] + WAVE [4]
    out.write_all(b"RIFF")?;
    
    // File Size = 4 (WAVE) + 8 (fmt hdr) + fmt_len + 8 (data hdr) + data_len
    let chunk_size = 4 + 8 + base_fmt.len() as u32 + 8 + total_data_size;
    out.write_all(&chunk_size.to_le_bytes())?;
    
    out.write_all(b"WAVE")?;
    
    // fmt chunk
    out.write_all(b"fmt ")?;
    out.write_all(&(base_fmt.len() as u32).to_le_bytes())?;
    out.write_all(&base_fmt)?;
    
    // data chunk header
    out.write_all(b"data")?;
    out.write_all(&total_data_size.to_le_bytes())?;
    
    // Stream data from source files
    for (i, info) in infos.iter().enumerate() {
        let mut input = File::open(&input_paths[i])?;
        input.seek(SeekFrom::Start(info.data_offset))?;
        
        let mut reader = input.take(info.data_size as u64);
        std::io::copy(&mut reader, &mut out)?;
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_dummy_wav(size: u32, sample_rate: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.write_all(b"RIFF").unwrap();
        let total_size = 36 + size;
        buf.write_all(&total_size.to_le_bytes()).unwrap();
        buf.write_all(b"WAVE").unwrap();
        
        buf.write_all(b"fmt ").unwrap();
        buf.write_all(&16u32.to_le_bytes()).unwrap();
        // PCM (1), Mono (1), SampleRate, ByteRate, BlockAlign (2), Bits (16)
        buf.write_all(&1u16.to_le_bytes()).unwrap(); 
        buf.write_all(&1u16.to_le_bytes()).unwrap();
        buf.write_all(&sample_rate.to_le_bytes()).unwrap();
        buf.write_all(&(sample_rate * 2).to_le_bytes()).unwrap();
        buf.write_all(&2u16.to_le_bytes()).unwrap();
        buf.write_all(&16u16.to_le_bytes()).unwrap();
        
        buf.write_all(b"data").unwrap();
        buf.write_all(&size.to_le_bytes()).unwrap();
        buf.write_all(&vec![0u8; size as usize]).unwrap();
        
        buf
    }

    #[test]
    fn test_merge_wav_files() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path1 = temp_dir.path().join("1.wav");
        let path2 = temp_dir.path().join("2.wav");
        let output = temp_dir.path().join("out.wav");
        
        let wav1 = create_dummy_wav(10, 44100);
        let wav2 = create_dummy_wav(20, 44100);
        
        std::fs::write(&path1, &wav1)?;
        std::fs::write(&path2, &wav2)?;
        
        merge_wav_files(&[path1.clone(), path2.clone()], &output)?;
        
        let info = scan_wav(&output)?;
        assert_eq!(info.data_size, 30);
        assert_eq!(info.fmt_content.len(), 16);
        
        Ok(())
    }

    #[test]
    fn test_merge_binary_files() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path1 = temp_dir.path().join("1.bin");
        let path2 = temp_dir.path().join("2.bin");
        let output = temp_dir.path().join("out.bin");
        
        std::fs::write(&path1, b"Hello")?;
        std::fs::write(&path2, b"World")?;
        
        merge_binary_files(&[path1, path2], &output)?;
        
        let content = std::fs::read(&output)?;
        assert_eq!(content, b"HelloWorld");
        Ok(())
    }
}
