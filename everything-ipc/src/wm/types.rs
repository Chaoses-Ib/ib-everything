use std::mem;

use derive_more::TryFrom;
use widestring::{U16CStr, u16cstr};
use windows::{Win32::Foundation::FILETIME, core::PCWSTR};

pub const EVERYTHING_IPC_GET_MINOR_VERSION: u32 = 1;

pub const EVERYTHING_IPC_COPYDATA_QUERY2W: u32 = 18;

pub const EVERYTHING_IPC_IS_DB_LOADED: u32 = 401;
pub const EVERYTHING_IPC_IS_FILE_INFO_INDEXED: u32 = 411;

/// Register window message for IPC creation
pub fn register_ipc_created_message() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW;

    unsafe { RegisterWindowMessageW(PCWSTR(u16cstr!("EVERYTHING_IPC_CREATED").as_ptr())) }
}

/// Search flags for Everything IPC
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SearchFlags(u32);

bitflags::bitflags! {
    impl SearchFlags: u32 {
        const MatchCase = 0x00000001;
        const MatchWholeWord = 0x00000002;
        const MatchPath = 0x00000004;
        const Regex = 0x00000008;
        /// Abandoned?
        const MatchAccents = 0x00000010;
    }
}

/// Sort types for Everything IPC
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, TryFrom)]
#[try_from(repr)]
pub enum Sort {
    /// Best performance
    #[default]
    NameAscending = 1,
    NameDescending = 2,
    PathAscending = 3,
    PathDescending = 4,
    SizeAscending = 5,
    SizeDescending = 6,
    ExtensionAscending = 7,
    ExtensionDescending = 8,
    TypeNameAscending = 9,
    TypeNameDescending = 10,
    DateCreatedAscending = 11,
    DateCreatedDescending = 12,
    DateModifiedAscending = 13,
    DateModifiedDescending = 14,
    AttributesAscending = 15,
    AttributesDescending = 16,
    FileListFilenameAscending = 17,
    FileListFilenameDescending = 18,
    RunCountAscending = 19,
    RunCountDescending = 20,
    DateRecentlyChangedAscending = 21,
    DateRecentlyChangedDescending = 22,
    DateAccessedAscending = 23,
    DateAccessedDescending = 24,
    DateRunAscending = 25,
    DateRunDescending = 26,
}

/// File info types for Everything
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileInfo {
    FileSize = 1,
    FolderSize = 2,
    DateCreated = 3,
    DateModified = 4,
    DateAccessed = 5,
    Attributes = 6,
}

/// Request flags for Everything IPC
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestFlags(u32);

bitflags::bitflags! {
    impl RequestFlags: u32 {
        const FileName = 0x00000001;
        const Path = 0x00000002;
        const FullPathAndFileName = 0x00000004;
        const Extension = 0x00000008;
        const Size = 0x00000010;
        const DateCreated = 0x00000020;
        const DateModified = 0x00000040;
        const DateAccessed = 0x00000080;
        const Attributes = 0x00000100;
        const FileListFileName = 0x00000200;
        const RunCount = 0x00000400;
        const DateRun = 0x00000800;
        const DateRecentlyChanged = 0x00001000;
        const HighlightedFileName = 0x00002000;
        const HighlightedPath = 0x00004000;
        const HighlightedFullPathAndFileName = 0x00008000;
    }
}

/// Request data type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestDataType {
    Str,
    Size,
    Date,
    Dword,
}

impl RequestFlags {
    /// Returns the data type for a given request flag
    pub fn data_type(self) -> RequestDataType {
        match self {
            Self::FileName
            | Self::Path
            | Self::FullPathAndFileName
            | Self::Extension
            | Self::FileListFileName
            | Self::HighlightedFileName
            | Self::HighlightedPath
            | Self::HighlightedFullPathAndFileName => RequestDataType::Str,
            Self::Size => RequestDataType::Size,
            Self::DateCreated
            | Self::DateModified
            | Self::DateAccessed
            | Self::DateRun
            | Self::DateRecentlyChanged => RequestDataType::Date,
            Self::Attributes | Self::RunCount => RequestDataType::Dword,
            _ => panic!("Invalid request flag"),
        }
    }
}

/// Query item value - the result of a query request
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryValue<'a> {
    /// String value (file name, path, extension, etc.)
    Str(&'a U16CStr),
    /// Size value (file size in bytes)
    Size(u64),
    /// Date value (FILETIME)
    Time(FILETIME),
    /// Dword value (attributes, run count)
    U32(u32),
}

/// Query results header from Everything
/// Matches `EVERYTHING_IPC_LIST2` from `Everything.h`
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EverythingIpcList2 {
    pub totitems: u32,      // found items
    pub numitems: u32,      // available items
    pub offset: u32,        // offset of the first result
    pub request_flags: u32, // valid request flags
    pub sort_type: u32,     // actual sort type
}

/// A single item in query results
/// Matches `EVERYTHING_IPC_ITEM2` from `Everything.h`
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EverythingIpcItem2 {
    /// TODO
    pub flags: u32,
    pub data_offset: u32,
}

/// Query request data for Everything IPC
/// Matches `EVERYTHING_IPC_QUERY2` from `Everything.h`
/// Note: The SDK uses a flexible array member, so we use a pointer-based approach
/// to avoid padding issues.
#[repr(C, packed)]
pub struct EverythingIpcQuery2 {
    pub reply_hwnd: u32,
    pub reply_copydata_message: u32,
    pub search_flags: u32,
    pub offset: u32,
    pub max_results: u32,
    pub request_flags: u32,
    pub sort_type: u32,
    /// Flexible array member, at least 1
    pub search_string: [u16; 1],
}

impl EverythingIpcQuery2 {
    /// Calculate the total size needed for a query with given search string length
    /// Header is exactly 28 bytes (7 DWORDs) + minimum 2 bytes for search_string
    pub fn size_for_search(search_len: usize) -> usize {
        mem::size_of::<Self>() + (search_len * mem::size_of::<u16>())
    }

    /// Create a new query request with the given parameters
    /// Returns the request buffer
    pub fn create(
        reply_hwnd: u32,
        reply_copydata_message: u32,
        search_flags: u32,
        offset: u32,
        max_results: u32,
        request_flags: u32,
        sort_type: u32,
        search: &str,
    ) -> Vec<u8> {
        let search_len = search.encode_utf16().count();
        let total_size = Self::size_for_search(search_len);

        let mut buffer = Vec::with_capacity(total_size);

        // Write the header fields (exactly 28 bytes)
        buffer.extend_from_slice(&reply_hwnd.to_le_bytes());
        buffer.extend_from_slice(&reply_copydata_message.to_le_bytes());
        buffer.extend_from_slice(&search_flags.to_le_bytes());
        buffer.extend_from_slice(&offset.to_le_bytes());
        buffer.extend_from_slice(&max_results.to_le_bytes());
        buffer.extend_from_slice(&request_flags.to_le_bytes());
        buffer.extend_from_slice(&sort_type.to_le_bytes());

        // Write the search string (UTF-16)
        // Always write at least 1 null terminator (2 bytes)
        for ch in search.encode_utf16() {
            buffer.extend_from_slice(&ch.to_le_bytes());
        }
        // Add null terminator
        buffer.extend_from_slice(&0u16.to_le_bytes());

        buffer
    }
}

/// A single query result item
pub struct QueryItem<'a> {
    request: RequestFlags,
    data: &'a [u8],
}

impl std::fmt::Debug for QueryItem<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("QueryItem");
        debug.field("request", &self.request);
        for (name, _flag, value) in self.items_with_name() {
            debug.field(name, &value);
        }
        debug.finish()
    }
}

impl QueryItem<'_> {
    pub(crate) fn new<'a>(request: RequestFlags, data: &'a [u8]) -> QueryItem<'a> {
        QueryItem { request, data }
    }

    /// Get raw data pointer for a request flag
    pub fn get_ptr(&self, flag: RequestFlags) -> Option<*const u8> {
        if (self.request.bits() & flag.bits()) == 0 {
            return None;
        }

        let mut offset = 0;
        for f in [
            RequestFlags::FileName,
            RequestFlags::Path,
            RequestFlags::FullPathAndFileName,
            RequestFlags::Extension,
            RequestFlags::Size,
            RequestFlags::DateCreated,
            RequestFlags::DateModified,
            RequestFlags::DateAccessed,
            RequestFlags::Attributes,
            RequestFlags::FileListFileName,
            RequestFlags::RunCount,
            RequestFlags::DateRun,
            RequestFlags::DateRecentlyChanged,
            RequestFlags::HighlightedFileName,
            RequestFlags::HighlightedPath,
            RequestFlags::HighlightedFullPathAndFileName,
        ] {
            if (self.request.bits() & f.bits()) == 0 {
                continue;
            }
            if f == flag {
                let ptr = unsafe { self.data.as_ptr().add(offset) };
                return Some(ptr);
            }
            match f.data_type() {
                RequestDataType::Str => {
                    let len = unsafe {
                        self.data
                            .as_ptr()
                            .add(offset)
                            .cast::<u32>()
                            .read_unaligned()
                    } as usize;
                    offset += mem::size_of::<u32>() + (len + 1) * mem::size_of::<u16>();
                }
                RequestDataType::Size => {
                    offset += mem::size_of::<u64>();
                }
                RequestDataType::Date => {
                    offset += mem::size_of::<FILETIME>();
                }
                RequestDataType::Dword => {
                    offset += mem::size_of::<u32>();
                }
            }
        }
        None
    }

    /// Get string value for a request flag
    pub fn get_str(&self, flag: RequestFlags) -> Option<&U16CStr> {
        let ptr = self.get_ptr(flag)?;
        let len = unsafe { ptr.cast::<u32>().read_unaligned() } as usize;
        let str_ptr = unsafe { ptr.add(mem::size_of::<u32>()) as *const u16 };
        Some(unsafe { U16CStr::from_ptr_unchecked(str_ptr, len) })
    }

    /// Get string value for a request flag
    ///
    /// Prefer [`QueryItem::get_str()`] when possible.
    pub fn get_string(&self, flag: RequestFlags) -> Option<String> {
        self.get_str(flag).map(|s| s.to_string_lossy())
    }

    /// Get size value for a request flag
    pub fn get_size(&self, flag: RequestFlags) -> Option<u64> {
        let ptr = self.get_ptr(flag)?;
        Some(unsafe { ptr.cast::<u64>().read_unaligned() })
    }

    /// Get date value for a request flag
    pub fn get_time(&self, flag: RequestFlags) -> Option<FILETIME> {
        let ptr = self.get_ptr(flag)?;
        Some(unsafe { ptr.cast::<FILETIME>().read_unaligned() })
    }

    /// Get dword value for a request flag
    pub fn get_u32(&self, flag: RequestFlags) -> Option<u32> {
        let ptr = self.get_ptr(flag)?;
        Some(unsafe { ptr.cast::<u32>().read_unaligned() })
    }

    /// Get value for a request flag as a QueryValue enum
    pub fn get(&self, flag: RequestFlags) -> Option<QueryValue<'_>> {
        match flag.data_type() {
            RequestDataType::Str => self.get_str(flag).map(QueryValue::Str),
            RequestDataType::Size => self.get_size(flag).map(QueryValue::Size),
            RequestDataType::Date => self.get_time(flag).map(QueryValue::Time),
            RequestDataType::Dword => self.get_u32(flag).map(QueryValue::U32),
        }
    }

    /// Iterator over all requested flags and their values
    pub fn keys(&self) -> impl Iterator<Item = RequestFlags> {
        self.request.iter()
    }

    /// Iterator over all requested flags and their values
    pub fn items(&self) -> impl Iterator<Item = (RequestFlags, QueryValue<'_>)> {
        self.request.iter().map(|r| (r, self.get(r).unwrap()))
    }

    /// Iterator over all requested flags and their values
    pub fn items_with_name(
        &self,
    ) -> impl Iterator<Item = (&'static str, RequestFlags, QueryValue<'_>)> {
        self.request
            .iter_names()
            .map(|(n, r)| (n, r, self.get(r).unwrap()))
    }
}

/// Query results from Everything
pub struct QueryList {
    id: u32,
    data: Box<[u8]>,
    offset: u32,
    len: u32,
    total_len: u32,
    request_flags: RequestFlags,
    sort: Sort,
}

impl std::fmt::Debug for QueryList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("QueryList");
        debug
            .field("id", &self.id)
            .field("len", &self.len)
            .field("offset", &self.offset)
            .field("total_len", &self.total_len)
            .field("request_flags", &self.request_flags)
            .field("sort", &self.sort);

        let items: Vec<_> = self.iter().collect();
        debug.field("items", &items).finish()
    }
}

impl QueryList {
    pub(crate) fn new(id: u32, data: Box<[u8]>) -> Self {
        // Parse header using EVERYTHING_IPC_LIST2 struct
        let header = unsafe { std::ptr::read(data.as_ptr() as *const EverythingIpcList2) };
        Self {
            id,
            data: data.into(),
            len: header.numitems,
            offset: header.offset,
            total_len: header.totitems,
            request_flags: RequestFlags::from_bits_truncate(header.request_flags),
            sort: header.sort_type.try_into().unwrap(),
        }
    }

    /// Get the query ID
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Check if results are empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the offset of the first result
    pub fn offset(&self) -> u32 {
        self.offset
    }

    /// Number of available items
    #[doc(alias = "available_num")]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Get the number of found items
    #[doc(alias = "found_num")]
    pub fn total_len(&self) -> usize {
        self.total_len as usize
    }

    /// Get the request flags
    pub fn request_flags(&self) -> RequestFlags {
        self.request_flags
    }

    /// Get the sort type
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Get item at index
    pub fn get(&self, index: usize) -> Option<QueryItem<'_>> {
        if index >= self.len() {
            return None;
        }

        // Header (EVERYTHING_IPC_LIST2) is 20 bytes
        // Items array starts at offset 20
        // Each item (EVERYTHING_IPC_ITEM2) is 8 bytes: flags(4) + data_offset(4)
        // data_offset in EVERYTHING_IPC_ITEM2 is relative to the start of the entire buffer

        // Get item at index using EVERYTHING_IPC_ITEM2 struct
        let items_offset = mem::size_of::<EverythingIpcList2>();
        let item_ptr = unsafe { self.data.as_ptr().add(items_offset + index * 8) };
        let item: EverythingIpcItem2 =
            unsafe { std::ptr::read(item_ptr as *const EverythingIpcItem2) };

        // data_offset is relative to the start of the entire buffer
        let data_offset = item.data_offset as usize;
        let data_ptr = unsafe { self.data.as_ptr().add(data_offset) };

        // Calculate data length from this offset to end of buffer
        let data_len = self.data.len() - data_offset;

        // Create a slice reference into the shared data (no allocation)
        let data_slice = unsafe { std::slice::from_raw_parts(data_ptr as *const u8, data_len) };

        Some(QueryItem::new(self.request_flags, data_slice))
    }

    /// Iterator over all items
    pub fn iter(&self) -> QueryListIter<'_> {
        QueryListIter {
            list: self,
            index: 0,
        }
    }
}

/// Iterator over query list
pub struct QueryListIter<'a> {
    list: &'a QueryList,
    index: usize,
}

impl<'a> Iterator for QueryListIter<'a> {
    type Item = QueryItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.list.get(self.index);
        self.index += 1;
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.list.len().saturating_sub(self.index);
        (size, Some(size))
    }
}

impl<'a> ExactSizeIterator for QueryListIter<'a> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_flags_combinations() {
        let flags = SearchFlags::MatchCase | SearchFlags::MatchWholeWord;
        assert_eq!(flags.bits(), 0x00000003);
    }

    #[test]
    fn request_flags_combinations() {
        let flags = RequestFlags::FileName | RequestFlags::Size | RequestFlags::DateModified;
        assert_eq!(flags.bits(), 0x00000051); // 1 | 16 | 64 = 81 = 0x51
    }

    #[test]
    fn request_flags_data_type() {
        assert_eq!(RequestFlags::FileName.data_type(), RequestDataType::Str);
        assert_eq!(RequestFlags::Size.data_type(), RequestDataType::Size);
        assert_eq!(RequestFlags::DateCreated.data_type(), RequestDataType::Date);
        assert_eq!(RequestFlags::Attributes.data_type(), RequestDataType::Dword);
    }

    #[test]
    fn sort_default() {
        assert_eq!(Sort::default(), Sort::NameAscending);
    }

    #[test]
    fn version_default() {
        let version: crate::Version = Default::default();
        assert_eq!(version.major, 0);
        assert_eq!(version.minor, 0);
        assert_eq!(version.revision, 0);
        assert_eq!(version.build, 0);
    }

    #[test]
    fn search_flags_not() {
        let flags = !SearchFlags::MatchCase;
        // bitflags::Not only inverts the defined bits, not the entire u32
        // For SearchFlags with 5 bits defined (0x1F), !MATCH_CASE = 0x1E = 30
        assert_eq!(flags.bits(), 0x1E);
    }

    #[test]
    fn request_flags_not() {
        let flags = !RequestFlags::FileName;
        // bitflags::Not only inverts the defined bits, not the entire u32
        // For RequestFlags with 16 bits defined (0xFFFF), !FILE_NAME = 0xFFFE = 65534
        assert_eq!(flags.bits(), 0xFFFE);
    }

    #[test]
    fn everything_ipc_query2_size() {
        // Test the struct size calculation
        // Header is exactly 28 bytes (7 DWORDs) + minimum 2 bytes for search_string

        // Test size_for_search with empty string (just the minimum 1 u16)
        assert_eq!(EverythingIpcQuery2::size_for_search(0), 30); // 30 bytes minimum

        // Test size_for_search with "test" (4 chars)
        let search_size = 30 + 4 * 2; // 30 minimum + 4 chars * 2 bytes per char
        assert_eq!(EverythingIpcQuery2::size_for_search(4), search_size);
    }

    #[test]
    fn everything_ipc_query2_create() {
        let buffer = EverythingIpcQuery2::create(
            1234,
            5678,
            0,
            0,
            10,
            RequestFlags::FileName.bits(),
            Sort::NameAscending as u32,
            "test",
        );

        // Minimum 30 bytes + 4 chars * 2 bytes = 38 bytes (30 = 28 header + 2 null terminator)
        assert_eq!(buffer.len(), 38);

        // Verify header fields
        assert_eq!(u32::from_le_bytes(buffer[0..4].try_into().unwrap()), 1234);
        assert_eq!(u32::from_le_bytes(buffer[4..8].try_into().unwrap()), 5678);
        assert_eq!(u32::from_le_bytes(buffer[8..12].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(buffer[12..16].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(buffer[16..20].try_into().unwrap()), 10);
        assert_eq!(
            u32::from_le_bytes(buffer[20..24].try_into().unwrap()),
            RequestFlags::FileName.bits()
        );
        assert_eq!(
            u32::from_le_bytes(buffer[24..28].try_into().unwrap()),
            Sort::NameAscending as u32
        );
    }

    // Helper to create minimal QueryList data
    // Note: EVERYTHING_IPC_LIST2 header is 20 bytes:
    //   - totitems (4) + numitems (4) + offset (4) + request_flags (4) + sort_type (4)
    fn make_query_data(
        found_num: u32,
        available_num: u32,
        request_flags: u32,
        sort: Sort,
        items: &[(u32, u32)],
        data: &[u8],
    ) -> Box<[u8]> {
        let mut result = Vec::new();
        result.extend_from_slice(&found_num.to_le_bytes());
        result.extend_from_slice(&available_num.to_le_bytes());
        result.extend_from_slice(&0u32.to_le_bytes()); // offset field (unused)
        result.extend_from_slice(&request_flags.to_le_bytes());
        result.extend_from_slice(&(sort as u32).to_le_bytes());

        // Write items
        for (flags, offset) in items {
            result.extend_from_slice(&flags.to_le_bytes());
            result.extend_from_slice(&offset.to_le_bytes());
        }

        // Write data
        result.extend_from_slice(data);
        result.into_boxed_slice()
    }

    #[test]
    fn query_list_empty() {
        // Create minimal valid data with zero available items
        // Header (EVERYTHING_IPC_LIST2): found_num(4) + available_num(4) + offset(4) + request_flags(4) + sort(4) = 20 bytes
        let data = make_query_data(0, 0, 0, Sort::default(), &[], b"");
        let results = QueryList::new(0, data);
        assert!(results.is_empty());
        assert_eq!(results.len(), 0);
        assert_eq!(results.total_len(), 0);
        assert!(results.get(0).is_none());
    }

    #[test]
    fn query_list_with_items() {
        // Create a query list with one file name
        // Header (EVERYTHING_IPC_LIST2): 20 bytes
        // Item (EVERYTHING_IPC_ITEM2): flags(4) + offset(4) = 8 bytes

        // For a simple test, we just verify the structure parsing works
        let data = make_query_data(
            1,
            1,
            RequestFlags::FileName.bits(),
            Sort::NameAscending,
            &[(RequestFlags::FileName.bits(), 20)], // offset 20 = start of data
            b"",
        );
        let results = QueryList::new(0, data);
        assert_eq!(results.total_len(), 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results.request_flags(), RequestFlags::FileName);
    }

    #[test]
    fn query_list_multiple_items() {
        let data = make_query_data(
            2,
            2,
            RequestFlags::FileName.bits(),
            Sort::NameAscending,
            &[
                (RequestFlags::FileName.bits(), 20), // offset 20 = start of data
                (RequestFlags::FileName.bits(), 28), // offset 28 = 20 + 8 (8 bytes for first item)
            ],
            b"",
        );
        let results = QueryList::new(0, data);
        assert_eq!(results.total_len(), 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn query_list_sort_parsing() {
        for (sort_value, expected_sort) in [
            (1, Sort::NameAscending),
            (2, Sort::NameDescending),
            (3, Sort::PathAscending),
            (4, Sort::PathDescending),
            (5, Sort::SizeAscending),
            (6, Sort::SizeDescending),
            (7, Sort::ExtensionAscending),
            (8, Sort::ExtensionDescending),
        ] {
            let data = make_query_data(0, 0, 0, expected_sort, &[], b"");
            let results = QueryList::new(0, data);
            assert_eq!(results.sort(), expected_sort, "sort value {}", sort_value);
        }
    }

    #[test]
    fn query_item_str() {
        // Create data with a UTF-16 string "test"
        // Header is 20 bytes, items array is 8 bytes (1 item)
        // So data starts at offset 20 + 8 = 28
        // Format: length (u32 LE) + string (u16 LE) + null terminator
        // "test" in UTF-16: [116, 0, 101, 0, 115, 0, 116, 0] (4 chars)
        // Plus null terminator = 5 u16s = 10 bytes
        // String data: length(4) + string(10) = 14 bytes
        let string_data: Vec<u8> = vec![4, 0, 0, 0, 116, 0, 101, 0, 115, 0, 116, 0, 0, 0];
        let data = make_query_data(
            1,
            1,
            RequestFlags::FileName.bits(),
            Sort::NameAscending,
            &[(RequestFlags::FileName.bits(), 28)], // offset 28 = 20 (header) + 8 (1 item)
            &string_data,
        );
        let results = QueryList::new(0, data);

        // Get the first item
        if let Some(item) = results.get(0) {
            if let Some(str_val) = item.get_str(RequestFlags::FileName) {
                assert_eq!(str_val.to_string_lossy(), "test");
            } else {
                panic!("Expected FILE_NAME string");
            }
        } else {
            panic!("Expected item at index 0");
        }
    }

    #[test]
    fn query_list_iter() {
        let data = make_query_data(
            2,
            2,
            RequestFlags::FileName.bits(),
            Sort::NameAscending,
            &[
                (RequestFlags::FileName.bits(), 20), // offset 20 = start of data
                (RequestFlags::FileName.bits(), 28), // offset 28 = 20 + 8 (8 bytes for first item)
            ],
            b"",
        );
        let results = QueryList::new(0, data);

        let mut count = 0;
        for _ in results.iter() {
            count += 1;
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn query_item_out_of_bounds() {
        let data = make_query_data(0, 0, 0, Sort::default(), &[], b"");
        let results = QueryList::new(0, data);
        assert!(results.get(0).is_none());
    }

    #[test]
    fn filetime_structure() {
        let ft = FILETIME {
            dwLowDateTime: 0x12345678,
            dwHighDateTime: 0x9ABCDEF0,
        };
        assert_eq!(ft.dwLowDateTime, 0x12345678);
        assert_eq!(ft.dwHighDateTime, 0x9ABCDEF0);
    }

    #[test]
    fn query_list_get_by_index() {
        let data = make_query_data(
            1,
            1,
            RequestFlags::FileName.bits(),
            Sort::NameAscending,
            &[(RequestFlags::FileName.bits(), 20)], // offset 20 = start of data
            b"",
        );
        let results = QueryList::new(0, data);

        assert!(results.get(0).is_some());
        assert!(results.get(1).is_none()); // Out of bounds
    }

    #[test]
    fn request_flags_all() {
        // Test all request flags have unique values
        let flags = [
            RequestFlags::FileName,
            RequestFlags::Path,
            RequestFlags::FullPathAndFileName,
            RequestFlags::Extension,
            RequestFlags::Size,
            RequestFlags::DateCreated,
            RequestFlags::DateModified,
            RequestFlags::DateAccessed,
            RequestFlags::Attributes,
            RequestFlags::FileListFileName,
            RequestFlags::RunCount,
            RequestFlags::DateRun,
            RequestFlags::DateRecentlyChanged,
            RequestFlags::HighlightedFileName,
            RequestFlags::HighlightedPath,
            RequestFlags::HighlightedFullPathAndFileName,
        ];

        let mut seen = std::collections::HashSet::new();
        for flag in flags {
            assert!(
                !seen.contains(&flag.0),
                "Duplicate flag value: {:#x}",
                flag.0
            );
            seen.insert(flag.0);
        }
    }
}
