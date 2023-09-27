# Motive
I want to read files on my USB disk, which was formatted to ext4. But macOS doesn't recognize it.

I don't want to start a virtual machine, nor do I want to install some kernel extension.

# Usage

- Build the crate

```
cargo build
```

- Identify the device, for example `/dev/disk4`

You can use `Disk Utility.app` for help.

- Run the program

```
sudo ./target/debug/ext4nfs /dev/disk4
```

`sudo` is required to access `/dev/disk4`.

- Mount the drive

```
mkdir temp
mount_nfs -o nolocks,vers=3,tcp,rsize=131072,actimeo=120,port=11111,mountport=11111 localhost:/ temp/
```

## Options

```
    --port <PORT>  [default: 11111]
```

# Major Dependencies

- [ext4](https://crates.io/crates/ext4) for reading ext4 partition
- [nfsserve](https://crates.io/crates/nfsserve) for serving the file system over NFS