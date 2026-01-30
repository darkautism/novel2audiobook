use anyhow::{anyhow, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Merges multiple audio files via simple binary concatenation.
/// Suitable for MP3 or other stream-based formats.
pub fn merge_binary_files(inputs: &mut [&mut dyn ReadSeek], output: &mut dyn Write) -> Result<()> {
    for (i, input) in inputs.iter_mut().enumerate() {
        input.seek(SeekFrom::Start(0))?;
        std::io::copy(*input, output).with_context(|| format!("Failed to copy input {}", i))?;
    }
    Ok(())
}

struct WavInfo {
    fmt_content: Vec<u8>,
    data_offset: u64,
    data_size: u32,
}

fn scan_wav(reader: &mut dyn ReadSeek) -> Result<WavInfo> {
    // Check RIFF
    let mut id = [0u8; 4];
    reader.read_exact(&mut id)?;
    if &id != b"RIFF" { return Err(anyhow!("Not a RIFF file")); }
    
    // Skip File Size
    reader.seek(SeekFrom::Current(4))?;
    
    // Check WAVE
    reader.read_exact(&mut id)?;
    if &id != b"WAVE" { return Err(anyhow!("Not a WAVE file")); }
    
    let mut fmt_content: Option<Vec<u8>> = None;
    let mut data_offset: Option<u64> = None;
    let mut data_size: Option<u32> = None;
    
    loop {
        let mut chunk_id = [0u8; 4];
        let n = reader.read(&mut chunk_id)?;
        if n == 0 { break; } // EOF
        if n < 4 { return Err(anyhow!("Unexpected EOF reading chunk ID")); }
        
        let mut size_buf = [0u8; 4];
        reader.read_exact(&mut size_buf)?;
        let chunk_size = u32::from_le_bytes(size_buf);
        
        if &chunk_id == b"fmt " {
            let mut buf = vec![0u8; chunk_size as usize];
            reader.read_exact(&mut buf)?;
            fmt_content = Some(buf);
        } else if &chunk_id == b"data" {
            data_offset = Some(reader.stream_position()?);
            data_size = Some(chunk_size);
            // We found data, stop scanning
            break; 
        } else {
            // Skip unknown chunk
            reader.seek(SeekFrom::Current(chunk_size as i64))?;
        }
    }
    
    Ok(WavInfo {
        fmt_content: fmt_content.ok_or_else(|| anyhow!("Missing fmt chunk"))?,
        data_offset: data_offset.ok_or_else(|| anyhow!("Missing data chunk"))?,
        data_size: data_size.ok_or_else(|| anyhow!("Missing data chunk size"))?,
    })
}

/// Merges multiple WAV files by parsing headers and concatenating data chunks.
/// Ensures all files have compatible format (fmt chunk).
pub fn merge_wav_files(inputs: &mut [&mut dyn ReadSeek], output: &mut dyn Write) -> Result<()> {
    if inputs.is_empty() {
        return Ok(());
    }

    // 1. Scan all files to gather info and calculate total size
    let mut total_data_size: u32 = 0;
    let mut infos = Vec::with_capacity(inputs.len());
    
    // Check first file
    inputs[0].seek(SeekFrom::Start(0))?;
    let first_info = scan_wav(inputs[0])?;
    let base_fmt = first_info.fmt_content.clone();
    
    total_data_size += first_info.data_size;
    infos.push(first_info);
    
    for (i, input) in inputs.iter_mut().enumerate().skip(1) {
        input.seek(SeekFrom::Start(0))?;
        let info = scan_wav(*input).with_context(|| format!("Failed to parse WAV input {}", i))?;
        
        // Verify format compatibility
        if info.fmt_content != base_fmt {
            return Err(anyhow!("WAV format mismatch in input {}. All segments must have same sample rate/channels.", i));
        }
        
        total_data_size += info.data_size;
        infos.push(info);
    }
    
    // 2. Write Output
    // RIFF [4] + Size [4] + WAVE [4]
    output.write_all(b"RIFF")?;
    
    // File Size = 4 (WAVE) + 8 (fmt hdr) + fmt_len + 8 (data hdr) + data_len
    let chunk_size = 4 + 8 + base_fmt.len() as u32 + 8 + total_data_size;
    output.write_all(&chunk_size.to_le_bytes())?;
    
    output.write_all(b"WAVE")?;
    
    // fmt chunk
    output.write_all(b"fmt ")?;
    output.write_all(&(base_fmt.len() as u32).to_le_bytes())?;
    output.write_all(&base_fmt)?;
    
    // data chunk header
    output.write_all(b"data")?;
    output.write_all(&total_data_size.to_le_bytes())?;
    
    // Stream data from source files
    for (i, info) in infos.iter().enumerate() {
        let input = &mut inputs[i];
        input.seek(SeekFrom::Start(info.data_offset))?;
        
        let mut reader = input.take(info.data_size as u64);
        std::io::copy(&mut reader, output)?;
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

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
        
        let mut f1 = File::open(&path1)?;
        let mut f2 = File::open(&path2)?;
        let mut out = File::create(&output)?;
        
        let mut inputs: Vec<&mut dyn ReadSeek> = vec![&mut f1, &mut f2];
        merge_wav_files(&mut inputs, &mut out)?;
        
        // Use scan_wav to verify output. But scan_wav takes reader.
        // We need to re-open output.
        drop(out); // Close file to flush
        let mut out_read = File::open(&output)?;
        let info = scan_wav(&mut out_read)?;
        
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
        
        let mut f1 = File::open(&path1)?;
        let mut f2 = File::open(&path2)?;
        let mut out = File::create(&output)?;
        
        let mut inputs: Vec<&mut dyn ReadSeek> = vec![&mut f1, &mut f2];
        merge_binary_files(&mut inputs, &mut out)?;
        
        let content = std::fs::read(&output)?;
        assert_eq!(content, b"HelloWorld");
        Ok(())
    }
}
