use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;

use crate::args::{FileMode, OutputMode};

pub struct OutputWriter {
    print_stdout: bool,
    file_writer: Option<FileWriter>,
}

enum FileWriter {
    Append { file: std::fs::File },
    Individual { dir: PathBuf, seq: u64 },
}

impl OutputWriter {
    pub fn new(
        output_mode: OutputMode,
        file_mode: FileMode,
        output_dir: &str,
        id: &str,
    ) -> Result<Self> {
        let print_stdout = matches!(output_mode, OutputMode::Stdout | OutputMode::Both);

        let file_writer = match output_mode {
            OutputMode::Stdout => None,
            OutputMode::File | OutputMode::Both => {
                Some(FileWriter::new(file_mode, output_dir, id)?)
            }
        };

        Ok(Self {
            print_stdout,
            file_writer,
        })
    }

    pub fn write_message(&mut self, msg: &str) -> Result<()> {
        if self.print_stdout {
            println!("{}", msg);
        }
        if let Some(ref mut fw) = self.file_writer {
            fw.write_message(msg)?;
        }
        Ok(())
    }
}

impl FileWriter {
    fn new(file_mode: FileMode, output_dir: &str, id: &str) -> Result<Self> {
        let dir = PathBuf::from(output_dir);
        std::fs::create_dir_all(&dir)?;

        match file_mode {
            FileMode::Append => {
                let path = dir.join(format!("{}.txt", id));
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)?;
                Ok(FileWriter::Append { file })
            }
            FileMode::Individual => {
                let sub_dir = dir.join(id);
                std::fs::create_dir_all(&sub_dir)?;
                Ok(FileWriter::Individual {
                    dir: sub_dir,
                    seq: 0,
                })
            }
        }
    }

    fn write_message(&mut self, msg: &str) -> Result<()> {
        match self {
            FileWriter::Append { file } => {
                writeln!(file, "{}", msg)?;
            }
            FileWriter::Individual { dir, seq } => {
                let n = *seq;
                *seq += 1;
                let filename = format!("{:0>6}.txt", n);
                let path = dir.join(&filename);
                std::fs::write(&path, msg)?;
            }
        }
        Ok(())
    }
}

impl Drop for FileWriter {
    fn drop(&mut self) {
        if let FileWriter::Append { file } = self {
            let _ = file.flush();
        }
    }
}
