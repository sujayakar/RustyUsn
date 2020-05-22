use std::io::Read;
use chrono::{DateTime, Utc};
use encoding::all::UTF_16LE;
use encoding::{DecoderTrap, Encoding};
use winstructs::ntfs::mft_reference::MftReference;
use byteorder::{ByteOrder, ReadBytesExt, LittleEndian};
use serde::ser::{SerializeStruct};
use serde::ser;
use serde::Serialize;
use serde_json::{Value};
use crate::flags;
use crate::error::UsnError;
use crate::utils::u64_to_datetime;


#[derive(Debug)]
pub struct UsnEntry {
    pub meta: EntryMeta,
    pub record: UsnRecord,
}
impl UsnEntry {
    pub fn new<R: Read>(meta: EntryMeta, version: u16, mut reader: R)-> Result<UsnEntry, UsnError>{
        let record = UsnRecord::new(
            version, 
            &mut reader
        )?;

        Ok(UsnEntry {
            meta: meta,
            record: record,
        })
    }

    pub fn to_json_value(&self) -> Result<Value, UsnError> {
        self.record.to_json_value(
            Some(
                self.meta.to_json_value()?
            )
        )
    }
}


/// EntryMeta is addon info describing where the UsnRecord was found.
///
#[derive(Serialize, Debug, Clone)]
pub struct EntryMeta {
    #[serde(rename(serialize = "meta__source"))]
    pub source: String,
    #[serde(rename(serialize = "meta__offset"))]
    pub offset: u64,
}
impl EntryMeta {
    pub fn new(source: &str, offset: u64) -> Self {
        EntryMeta {
            source: source.to_string(),
            offset: offset,
        }
    }

    pub fn to_json_value(&self) -> Result<Value, UsnError> {
        Ok(serde_json::to_value(&self)?)
    }
}


/// UsnRecord represents the multiple possible versions of the UsnRecord
#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum UsnRecord {
    V2(UsnRecordV2),
    V3(UsnRecordV3),
    V4(UsnRecordV4),
}
impl UsnRecord {
    pub fn new<R: Read>(version: u16, mut reader: R)-> Result<UsnRecord, UsnError> {
        if version == 2 {
            let usn_record_v2 = UsnRecordV2::new(
                &mut reader
            )?;
            Ok(UsnRecord::V2(usn_record_v2))
        } 
        else if version == 3 {
            let usn_record_v3 = UsnRecordV3::new(
                &mut reader
            )?;
            Ok(UsnRecord::V3(usn_record_v3))
        }
        else if version == 4 {
            let usn_record_v4 = UsnRecordV4::new(&mut reader)?;
            Ok(UsnRecord::V4(usn_record_v4))
        }
        else {
            Err(UsnError::unsupported_usn_version(
                format!("Unsupported USN version {}", version)
            ))
        }
    }

    pub fn get_usn(&self) -> u64 {
        match self {
            UsnRecord::V2(ref record) => record.usn.clone(),
            UsnRecord::V3(ref record) => record.usn.clone(),
            UsnRecord::V4(ref record) => record.usn.clone(),
        }
    }

    pub fn get_file_name(&self) -> Option<&str> {
        match self {
            UsnRecord::V2(ref record) => Some(&record.file_name),
            UsnRecord::V3(ref record) => Some(&record.file_name),
            UsnRecord::V4(..) => None,
        }
    }

    pub fn get_file_attributes(&self) -> Option<flags::FileAttributes> {
        match self {
            UsnRecord::V2(record) => Some(record.file_attributes),
            UsnRecord::V3(record) => Some(record.file_attributes),
            UsnRecord::V4(..) => None,
        }
    }

    pub fn get_reason_code(&self) -> flags::Reason {
        match self {
            UsnRecord::V2(record) => record.reason,
            UsnRecord::V3(record) => record.reason,
            UsnRecord::V4(record) => record.reason,
        }
    }

    pub fn get_file_reference(&self) -> MftReference {
        match self {
            UsnRecord::V2(record) => record.file_reference,
            UsnRecord::V3(record) => record.file_reference.as_mft_reference(),
            UsnRecord::V4(record) => record.file_reference.as_mft_reference(),
        }
    }

    pub fn get_parent_reference(&self) -> MftReference {
        match self {
            UsnRecord::V2(record) => record.parent_reference,
            UsnRecord::V3(record) => record.parent_reference.as_mft_reference(),
            UsnRecord::V4(record) => record.parent_reference.as_mft_reference(),
        }
    }

    pub fn to_json_value(&self, additional: Option<Value>) -> Result<Value, UsnError> {
        let mut this_value = serde_json::to_value(&self)?;

        match additional {
            Some(additional_value) => {
                let value_map = match this_value.as_object_mut() {
                    Some(map) => map,
                    None => return Err(
                        UsnError::json_value_error(
                            format!("Record json value's object is none. {:?}", self)
                        )
                    )
                };

                let additional_map = match additional_value.as_object() {
                    Some(map) => map.to_owned(),
                    None => return Err(
                        UsnError::json_value_error(
                            format!("additional value's object is none. {:?}", additional_value)
                        )
                    )
                };

                value_map.extend(additional_map);
            },
            None => {}
        }

        Ok(this_value)
    }
}


/// Represents a USN_RECORD_V2 structure
/// https://docs.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2
///
#[derive(Serialize, Debug)]
pub struct UsnRecordV2 {
    pub record_length: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub file_reference: MftReference,
    pub parent_reference: MftReference,
    pub usn: u64,
    pub timestamp: DateTime<Utc>,
    pub reason: flags::Reason,
    pub source_info: flags::SourceInfo,
    pub security_id: u32,
    pub file_attributes: flags::FileAttributes,
    pub file_name_length: u16,
    pub file_name_offset: u16,
    pub file_name: String
}

impl UsnRecordV2 {
    pub fn new<T: Read>(mut buffer: T) -> Result<UsnRecordV2, UsnError> {
        let record_length = buffer.read_u32::<LittleEndian>()?;

        // Do some length checks
        if record_length == 0 {
            return Err(
                UsnError::invalid_v2_record(
                    "Record length is 0.".to_string()
                )
            );
        }
        if record_length > 1024 {
            return Err(
                UsnError::invalid_v2_record(
                    "Record length is over 1024.".to_string()
                )
            );
        }

        let major_version = buffer.read_u16::<LittleEndian>()?;
        if major_version != 2 {
            return Err(
                UsnError::invalid_v2_record(
                    "Major version is not 2".to_string()
                )
            );
        }

        let minor_version = buffer.read_u16::<LittleEndian>()?;
        if minor_version != 0 {
            return Err(
                UsnError::invalid_v2_record(
                    "Minor version is not 0".to_string()
                )
            );
        }

        let file_reference = MftReference::from_reader(&mut buffer)?;
        let parent_reference = MftReference::from_reader(&mut buffer)?;
        let usn = buffer.read_u64::<LittleEndian>()?;
        let timestamp = u64_to_datetime(
            buffer.read_u64::<LittleEndian>()?
        );
        let reason = flags::Reason::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let source_info = flags::SourceInfo::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let security_id = buffer.read_u32::<LittleEndian>()?;
        let file_attributes = flags::FileAttributes::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let file_name_length = buffer.read_u16::<LittleEndian>()?;
        let file_name_offset = buffer.read_u16::<LittleEndian>()?;

        let mut name_buffer = vec![0; file_name_length as usize];
        buffer.read_exact(&mut name_buffer)?;

        let file_name = match UTF_16LE.decode(&name_buffer, DecoderTrap::Ignore) {
            Ok(file_name) => file_name,
            Err(error) => {
                return Err(UsnError::utf16_decode_error(
                    format!(
                        "Error Decoding Name [hex buffer: {}]: {:?}", 
                        hex::encode(&name_buffer), 
                        error
                    )
                ));
            },
        };

        Ok(
            UsnRecordV2 {
                record_length,
                major_version,
                minor_version,
                file_reference,
                parent_reference,
                usn,
                timestamp,
                reason,
                source_info,
                security_id,
                file_attributes,
                file_name_length,
                file_name_offset,
                file_name
            }
        )
    }
}


/// Represents a 128 bit file reference
///
#[derive(Debug)]
pub struct Ntfs128Reference(pub u128);

impl Ntfs128Reference {
    pub fn as_u128(&self) -> u128 {
        self.0
    }

    pub fn as_mft_reference(&self) -> MftReference {
        MftReference::from(
            LittleEndian::read_u64(
                &self.0.to_le_bytes()[0..8]
            )
        )
    }
}

impl ser::Serialize for Ntfs128Reference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let mut state = serializer.serialize_struct("Ntfs128Reference", 3)?;
        state.serialize_field("u128", &self.as_u128().to_string())?;
        let mft_reference = self.as_mft_reference();
        state.serialize_field("entry", &mft_reference.entry)?;
        state.serialize_field("sequence", &mft_reference.sequence)?;
        state.end()
    }
}

/// Represents a USN_RECORD_V3 structure
/// https://docs.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v3
///
#[derive(Serialize, Debug)]
pub struct UsnRecordV3 {
    pub record_length: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub file_reference: Ntfs128Reference,
    pub parent_reference: Ntfs128Reference,
    pub usn: u64,
    pub timestamp: DateTime<Utc>,
    pub reason: flags::Reason,
    pub source_info: flags::SourceInfo,
    pub security_id: u32,
    pub file_attributes: flags::FileAttributes,
    pub file_name_length: u16,
    pub file_name_offset: u16,
    pub file_name: String
}
impl UsnRecordV3 {
    pub fn new<T: Read>(mut buffer: T) -> Result<UsnRecordV3, UsnError> {
        let record_length = buffer.read_u32::<LittleEndian>()?;

        // Do some length checks
        if record_length == 0 {
            return Err(
                UsnError::invalid_record(
                    "Record length is 0.".to_string()
                )
            );
        }
        if record_length > 1024 {
            return Err(
                UsnError::invalid_record(
                    "Record length is over 1024.".to_string()
                )
            );
        }

        let major_version = buffer.read_u16::<LittleEndian>()?;
        if major_version != 3 {
            return Err(
                UsnError::invalid_record(
                    "Major version is not 3".to_string()
                )
            );
        }

        let minor_version = buffer.read_u16::<LittleEndian>()?;
        if minor_version != 0 {
            return Err(
                UsnError::invalid_record(
                    "Minor version is not 0".to_string()
                )
            );
        }

        let file_reference = Ntfs128Reference(
            buffer.read_u128::<LittleEndian>()?
        );
        let parent_reference = Ntfs128Reference(
            buffer.read_u128::<LittleEndian>()?
        );

        let usn = buffer.read_u64::<LittleEndian>()?;
        let timestamp = u64_to_datetime(
            buffer.read_u64::<LittleEndian>()?
        );
        let reason = flags::Reason::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let source_info = flags::SourceInfo::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let security_id = buffer.read_u32::<LittleEndian>()?;
        let file_attributes = flags::FileAttributes::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let file_name_length = buffer.read_u16::<LittleEndian>()?;
        let file_name_offset = buffer.read_u16::<LittleEndian>()?;

        let mut name_buffer = vec![0; file_name_length as usize];
        buffer.read_exact(&mut name_buffer)?;

        let file_name = match UTF_16LE.decode(&name_buffer, DecoderTrap::Ignore) {
            Ok(file_name) => file_name,
            Err(error) => {
                return Err(UsnError::utf16_decode_error(
                    format!(
                        "Error Decoding Name [hex buffer: {}]: {:?}", 
                        hex::encode(&name_buffer), 
                        error
                    )
                ));
            },
        };

        Ok(
            UsnRecordV3 {
                record_length,
                major_version,
                minor_version,
                file_reference,
                parent_reference,
                usn,
                timestamp,
                reason,
                source_info,
                security_id,
                file_attributes,
                file_name_length,
                file_name_offset,
                file_name
            }
        )
    }
}

#[derive(Serialize, Debug)]
pub struct UsnRecordCommonHeader {
    pub record_length: u32,
    pub major_version: u16,
    pub minor_version: u16,
}

#[derive(Serialize, Debug)]
pub struct UsnRecordExtent {
    pub offset: i64,
    pub length: i64,
}

/// Represents a USN_RECORD_V4 structure
/// https://docs.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v4
#[derive(Serialize, Debug)]
pub struct UsnRecordV4 {
    pub header: UsnRecordCommonHeader,
    pub file_reference: Ntfs128Reference,
    pub parent_reference: Ntfs128Reference,
    pub usn: u64,
    pub reason: flags::Reason,
    pub source_info: flags::SourceInfo,
    pub remaining_extents: u32,
    pub extents: Vec<UsnRecordExtent>,
}

impl UsnRecordV4 {
    pub fn new<T: Read>(mut buffer: T) -> Result<Self, UsnError> {
        // FIXME: We don't use this correctly to advance to the next record.
        let record_length = buffer.read_u32::<LittleEndian>()?;

        // FIXME: Compute the record length upper bound correctly.
        // https://docs.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v2
        if record_length == 0 || record_length > 1024 {
            return Err(UsnError::invalid_record(format!("Invalid length: {}", record_length)));
        }
        let major_version = buffer.read_u16::<LittleEndian>()?;
        if major_version != 4 {
            return Err(UsnError::invalid_record(format!("Unexpected version: {}", major_version)));
        }
        // Per https://docs.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-usn_record_v4#remarks,
        // having a higher minor version number allows for new fields before the variable length filename,
        // but existing fields remain valid.
        // FIXME: Add a lower bound to the minor version.
        let minor_version = buffer.read_u16::<LittleEndian>()?;

        // NB: FILE_ID_128 is defined as `BYTE Identifier[16]` and does not have 128-bit alignment.
        let file_reference = Ntfs128Reference(buffer.read_u128::<LittleEndian>()?);
        let parent_reference = Ntfs128Reference(buffer.read_u128::<LittleEndian>()?);

        let usn = buffer.read_u64::<LittleEndian>()?;

        let reason = flags::Reason::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);
        let source_info = flags::SourceInfo::from_bits_truncate(buffer.read_u32::<LittleEndian>()?);

        let remaining_extents = buffer.read_u32::<LittleEndian>()?;
        let number_of_extents = buffer.read_u16::<LittleEndian>()?;
        let extent_size = buffer.read_u16::<LittleEndian>()?;
        assert_eq!(extent_size, 16, "FIXME: Forwards compatibility");

        let mut extents = Vec::with_capacity(number_of_extents as usize);
        for _ in 0..number_of_extents {
            let offset = buffer.read_i64::<LittleEndian>()?;
            let length = buffer.read_i64::<LittleEndian>()?;
            extents.push(UsnRecordExtent { offset, length });
        }

        Ok(UsnRecordV4 {
            header: UsnRecordCommonHeader {
                record_length,
                major_version,
                minor_version,
            },
            file_reference,
            parent_reference,
            usn,
            reason,
            source_info,
            remaining_extents,
            extents,
        })
    }
}