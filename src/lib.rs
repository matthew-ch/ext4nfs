use std::{fs, io::{Seek, self, Read}};
use async_trait::async_trait;
use ext4::{SuperBlock, Options, Checksums, Enhanced};
use nfsserve::{vfs, nfs};
use tracing::{info, debug, error};

fn map_file_type(from: &ext4::FileType) -> nfs::ftype3 {
    match from {
        ext4::FileType::RegularFile => nfs::ftype3::NF3REG,
        ext4::FileType::SymbolicLink => nfs::ftype3::NF3LNK,
        ext4::FileType::CharacterDevice => nfs::ftype3::NF3CHR,
        ext4::FileType::BlockDevice => nfs::ftype3::NF3BLK,
        ext4::FileType::Directory => nfs::ftype3::NF3DIR,
        ext4::FileType::Fifo => nfs::ftype3::NF3FIFO,
        ext4::FileType::Socket => nfs::ftype3::NF3SOCK,
    }
}

fn map_time(from: &ext4::Time) -> nfs::nfstime3 {
    nfs::nfstime3 { seconds: from.epoch_secs, nseconds: from.nanos.unwrap_or(0) }
}

pub struct Ext4FS {
    super_block: SuperBlock<fs::File>,
}

impl Ext4FS {
    pub fn new_with_path(path: &str) -> Self {
        let file = fs::File::open(path).expect("path is not an openable file");
        let options = Options { checksums: Checksums::Enabled };
        Self {
            super_block: SuperBlock::new_with_options(file, &options).expect("did not find a valid ext4 volume")
        }
    }

    fn getattr_sync(&self, id: nfs::fileid3) -> Result<nfs::fattr3, nfs::nfsstat3> {
        let inode = self.super_block.load_inode(id as u32).map_err(|e| {
            error!("lookup error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })?;
        let stat = &inode.stat;
        Ok(nfs::fattr3 {
            fileid: id, 
            ftype: map_file_type(&stat.extracted_type),
            mode: stat.file_mode as u32,
            nlink: stat.link_count as u32,
            uid: stat.uid,
            gid: stat.gid,
            size: stat.size,
            used: 0,
            rdev: Default::default(),
            fsid: 0,
            atime: map_time(&stat.atime),
            mtime: map_time(&stat.mtime),
            ctime: map_time(&stat.ctime)
        })
    }
}

#[async_trait]
impl vfs::NFSFileSystem for Ext4FS {
    fn root_dir(&self) -> nfs::fileid3 {
        info!(func = "query root dir");
        self.super_block.root().unwrap().number as nfs::fileid3
    }

    fn capabilities(&self) -> vfs::VFSCapabilities {
        info!(func = "query capabilities");
        vfs::VFSCapabilities::ReadOnly
    }
    
    async fn write(&self, _id: nfs::fileid3, _offset: u64, _data: &[u8]) -> Result<nfs::fattr3, nfs::nfsstat3> {
        info!(func = "write");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn create(&self, _dirid: nfs::fileid3, _filename: &nfs::filename3, _attr: nfs::sattr3) -> Result<(nfs::fileid3, nfs::fattr3), nfs::nfsstat3> {
        info!(func = "create");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn create_exclusive(&self,  _dirid: nfs::fileid3, _filename: &nfs::filename3) -> Result<nfs::fileid3, nfs::nfsstat3> {
        info!(func = "create exclusive");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn getattr(&self, id: nfs::fileid3) -> Result<nfs::fattr3, nfs::nfsstat3> {
        info!(func = "getattr", id);
        self.getattr_sync(id)
    }

    async fn setattr(&self, _id: nfs::fileid3, _setattr: nfs::sattr3) -> Result<nfs::fattr3, nfs::nfsstat3> {
        info!(func = "setattr");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn lookup(&self, dirid: nfs::fileid3, filename: &nfs::filename3) -> Result<nfs::fileid3, nfs::nfsstat3> {
        info!(func = "lookup", dirid, ?filename);
        let dir = self.super_block.load_inode(dirid as u32).map_err(|e| {
            error!("lookup error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })?;
        if let Enhanced::Directory(entries) = self.super_block.enhance(&dir).map_err(|e| {
            error!("lookup error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })? {
            if let Some(entry) = entries.into_iter().find(|entry| entry.name.as_bytes() == &filename.0) {
                Ok(entry.inode as nfs::fileid3)
            } else {
                Err(nfs::nfsstat3::NFS3ERR_NOENT)
            }
        } else {
            Err(nfs::nfsstat3::NFS3ERR_NOTDIR)
        }
    }

    async fn read(&self, id: nfs::fileid3, offset: u64, count: u32) -> Result<(Vec<u8>, bool), nfs::nfsstat3> {
        info!(func = "read", id, offset, count);
        let inode = self.super_block.load_inode(id as u32).map_err(|e| {
            error!("read error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })?;
        let mut reader = self.super_block.open(&inode).map_err(|e| {
            error!("read error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })?;
        reader.seek(io::SeekFrom::Start(offset)).map_err(|e| {
            error!("read error: {}", e);
            nfs::nfsstat3::NFS3ERR_IO
        })?;
        let mut data = vec![0; count as usize];
        let read_count = reader.read(&mut data).map_err(|e| {
            error!("read error: {}", e);
            nfs::nfsstat3::NFS3ERR_IO
        })?;
        data.truncate(read_count);
        Ok((data, read_count as u64 + offset < inode.stat.size))
    }

    async fn readdir(&self, dirid: nfs::fileid3, start_after: nfs::fileid3, max_entries: usize) -> Result<vfs::ReadDirResult, nfs::nfsstat3> {
        info!(func = "readdir", dirid, start_after, max_entries);
        let dir = self.super_block.load_inode(dirid as u32).map_err(|e| {
            error!("lookup error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })?;
        if let Enhanced::Directory(entries) = self.super_block.enhance(&dir).map_err(|e| {
            error!("lookup error: {}", e);
            nfs::nfsstat3::NFS3ERR_BADHANDLE
        })? {
            let mut start_index = 0;
            if start_after > 0 {
                if let Some(pos) = entries.iter().position(|entry| entry.inode == start_after as u32) {
                    start_index = pos + 1;
                } else {
                    return Err(nfs::nfsstat3::NFS3ERR_BAD_COOKIE);
                }
            }
            let remaining_length = entries.len() - start_index;
            let mut ret = vfs::ReadDirResult {
                entries: Vec::new(),
                end: remaining_length <= max_entries
            };
            for entry in entries[start_index..].iter().take(max_entries) {
                ret.entries.push(vfs::DirEntry {
                    fileid: entry.inode as nfs::fileid3,
                    name: nfs::nfsstring(entry.name.clone().into_bytes()),
                    attr: self.getattr_sync(entry.inode as nfs::fileid3)?,
                });
            }
            debug!("readdir read {} entries", ret.entries.len());
            Ok(ret)
        } else {
            Err(nfs::nfsstat3::NFS3ERR_NOTDIR)
        }
    }

    async fn remove(&self, _dirid: nfs::fileid3, _filename: &nfs::filename3) -> Result<(), nfs::nfsstat3> {
        info!(func = "remove");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn rename(&self, _from_dirid: nfs::fileid3, _from_file_name: &nfs::filename3, _to_dirid: nfs::fileid3, _to_filename: &nfs::filename3) -> Result<(), nfs::nfsstat3> {
        info!(func = "rename");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn mkdir(&self, _dirid: nfs::fileid3, _dirname: &nfs::filename3) -> Result<(nfs::fileid3, nfs::fattr3), nfs::nfsstat3> {
        info!(func = "mkdir");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn symlink(&self, _dirid: nfs::fileid3, _linkname: &nfs::filename3, _symlink: &nfs::nfspath3, _attr: &nfs::sattr3) -> Result<(nfs::fileid3, nfs::fattr3), nfs::nfsstat3> {
        info!(func = "symlink");
        Err(nfs::nfsstat3::NFS3ERR_ROFS)
    }

    async fn readlink(&self, _id: nfs::fileid3) -> Result<nfs::nfspath3, nfs::nfsstat3> {
        info!(func = "readlink");
        Err(nfs::nfsstat3::NFS3ERR_NOTSUPP)
    }
}