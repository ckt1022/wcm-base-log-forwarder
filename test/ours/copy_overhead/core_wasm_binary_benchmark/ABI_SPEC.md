# Direct-memory ABI specification

All integers and offsets are little-endian. Offsets are absolute from the
start of the request buffer. Host code fills the guest allocation directly.

## Parse input

```text
record_count:u32
record_descriptor[record_count] = { offset:u32, length:u32 }
record UTF-8 payload bytes
```

The guest iterates borrowed `&str` views and passes each view directly to
`serde_json::from_str`; it never constructs an input `Vec<String>`.

## Filter and format input

```text
header = { entry_count:u32, total_tag_count:u32 }

entry_descriptor[entry_count] (36 bytes each):
  id:u64
  timestamp_offset:u32, timestamp_length:u32
  level:u8, padding[3]
  message_offset:u32, message_length:u32
  first_tag_index:u32, tag_count:u32

tag_descriptor[total_tag_count] (16 bytes each):
  key_offset:u32, key_length:u32
  value_offset:u32, value_length:u32

UTF-8 payload bytes
```

The guest uses borrowed entry and tag iterators. Filter reads only `id` and
`level`; format encodes directly from borrowed string views.

## Output

Parse returns the compact binary `ParsedEntry` list; filter returns a count
followed by `(id:u64, keep:u8)` elements; format returns the original framed
bytes. The host performs one direct slice copy from guest memory, frees both
allocations, and checks the raw bytes against a precomputed reference after H3.

```text
alloc(len:u32) -> ptr:u32              # uninitialized allocation
dealloc(ptr:u32, len:u32)
process(ptr:u32, len:u32) -> (output_ptr << 32) | output_len
```
