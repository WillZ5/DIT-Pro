//! ASC MHL XML Writer & Parser — Manifest and chain file I/O.
//!
//! Generates ASC MHL v2.0 compliant XML using `quick-xml`.
//!
//! Manifest structure:
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
//!   <creatorinfo> ... </creatorinfo>
//!   <processinfo> ... </processinfo>
//!   <hashes> ... </hashes>
//! </hashlist>
//! ```
//!
//! Chain file structure:
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
//!   <hashlist sequencenr="1">
//!     <path>0001_MediaRoot_2024-01-15_120000Z.mhl</path>
//!     <sha256>...</sha256>
//!   </hashlist>
//! </ascmhldirectory>
//! ```

use anyhow::{Context, Result};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use std::io::Cursor;
use std::path::Path;

use crate::hash_engine::HashAlgorithm;

use super::{
    MhlChainEntry, MhlGeneration, MhlHashEntry, CHAIN_NAMESPACE, MHL_NAMESPACE,
};

/// Convert a HashAlgorithm to the ASC MHL XML element name
fn algorithm_to_xml_name(algo: &HashAlgorithm) -> &'static str {
    match algo {
        HashAlgorithm::XXH64 => "xxh64",
        HashAlgorithm::XXH3 => "xxh3",
        HashAlgorithm::XXH128 => "xxh128",
        HashAlgorithm::SHA256 => "sha256",
        HashAlgorithm::MD5 => "md5",
    }
}

/// Write a complete ASC MHL manifest XML file.
pub async fn write_manifest(path: &Path, generation: &MhlGeneration) -> Result<()> {
    let xml_bytes = generate_manifest_xml(generation)?;
    tokio::fs::write(path, &xml_bytes)
        .await
        .with_context(|| format!("Failed to write manifest: {:?}", path))?;
    Ok(())
}

/// Generate manifest XML bytes in memory.
fn generate_manifest_xml(generation: &MhlGeneration) -> Result<Vec<u8>> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

    // XML declaration
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    // <hashlist version="2.0" xmlns="urn:ASC:MHL:v2.0">
    let mut hashlist = BytesStart::new("hashlist");
    hashlist.push_attribute(("version", "2.0"));
    hashlist.push_attribute(("xmlns", MHL_NAMESPACE));
    writer.write_event(Event::Start(hashlist))?;

    // <creatorinfo>
    write_creator_info(&mut writer, generation)?;

    // <processinfo>
    write_process_info(&mut writer, generation)?;

    // <hashes>
    write_hashes(&mut writer, generation)?;

    // </hashlist>
    writer.write_event(Event::End(BytesEnd::new("hashlist")))?;

    let result = writer.into_inner().into_inner();
    Ok(result)
}

/// Write <creatorinfo> section
fn write_creator_info(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    generation: &MhlGeneration,
) -> Result<()> {
    let info = &generation.creator_info;

    writer.write_event(Event::Start(BytesStart::new("creatorinfo")))?;

    // <creationdate>
    write_text_element(
        writer,
        "creationdate",
        &generation.creation_date.to_rfc3339(),
    )?;

    // <hostname>
    if let Some(ref hostname) = info.hostname {
        write_text_element(writer, "hostname", hostname)?;
    }

    // <tool version="...">name</tool>
    let mut tool_elem = BytesStart::new("tool");
    tool_elem.push_attribute(("version", info.tool_version.as_str()));
    writer.write_event(Event::Start(tool_elem))?;
    writer.write_event(Event::Text(BytesText::new(&info.tool_name)))?;
    writer.write_event(Event::End(BytesEnd::new("tool")))?;

    // <location>
    if let Some(ref location) = info.location {
        write_text_element(writer, "location", location)?;
    }

    // <comment>
    if let Some(ref comment) = info.comment {
        write_text_element(writer, "comment", comment)?;
    }

    // <author> entries
    for author in &info.authors {
        writer.write_event(Event::Start(BytesStart::new("author")))?;
        write_text_element(writer, "name", &author.name)?;
        if let Some(ref email) = author.email {
            write_text_element(writer, "email", email)?;
        }
        if let Some(ref phone) = author.phone {
            write_text_element(writer, "phone", phone)?;
        }
        if let Some(ref role) = author.role {
            write_text_element(writer, "role", role)?;
        }
        writer.write_event(Event::End(BytesEnd::new("author")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("creatorinfo")))?;
    Ok(())
}

/// Write <processinfo> section
fn write_process_info(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    generation: &MhlGeneration,
) -> Result<()> {
    writer.write_event(Event::Start(BytesStart::new("processinfo")))?;

    // <process>
    write_text_element(writer, "process", &generation.process_type.to_string())?;

    // <roothash>
    if generation.root_content_hash.is_some() || generation.root_structure_hash.is_some() {
        let hash_algo_name = algorithm_to_xml_name(&generation.hash_algorithm);
        let hashdate = generation.creation_date.to_rfc3339();

        writer.write_event(Event::Start(BytesStart::new("roothash")))?;

        // <content>
        if let Some(ref content_hash) = generation.root_content_hash {
            writer.write_event(Event::Start(BytesStart::new("content")))?;
            write_hash_element(writer, hash_algo_name, content_hash, None, Some(&hashdate))?;
            writer.write_event(Event::End(BytesEnd::new("content")))?;
        }

        // <structure>
        if let Some(ref structure_hash) = generation.root_structure_hash {
            writer.write_event(Event::Start(BytesStart::new("structure")))?;
            write_hash_element(
                writer,
                hash_algo_name,
                structure_hash,
                None,
                Some(&hashdate),
            )?;
            writer.write_event(Event::End(BytesEnd::new("structure")))?;
        }

        writer.write_event(Event::End(BytesEnd::new("roothash")))?;
    }

    // <ignore>
    if !generation.ignore_patterns.is_empty() {
        writer.write_event(Event::Start(BytesStart::new("ignore")))?;
        for pattern in &generation.ignore_patterns {
            write_text_element(writer, "pattern", pattern)?;
        }
        writer.write_event(Event::End(BytesEnd::new("ignore")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("processinfo")))?;
    Ok(())
}

/// Write <hashes> section with all file entries
fn write_hashes(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    generation: &MhlGeneration,
) -> Result<()> {
    writer.write_event(Event::Start(BytesStart::new("hashes")))?;

    // File hash entries
    for entry in &generation.hash_entries {
        write_hash_entry(writer, entry)?;
    }

    // Directory hash entries
    for dir_entry in &generation.directory_hashes {
        let hash_algo_name = algorithm_to_xml_name(&dir_entry.hash_algorithm);
        let hashdate = dir_entry.hash_date.to_rfc3339();

        writer.write_event(Event::Start(BytesStart::new("directoryhash")))?;

        // <path lastmodificationdate="...">
        let mut path_elem = BytesStart::new("path");
        path_elem.push_attribute((
            "lastmodificationdate",
            dir_entry.last_modified.to_rfc3339().as_str(),
        ));
        writer.write_event(Event::Start(path_elem))?;
        writer.write_event(Event::Text(BytesText::new(&dir_entry.path)))?;
        writer.write_event(Event::End(BytesEnd::new("path")))?;

        // <content>
        writer.write_event(Event::Start(BytesStart::new("content")))?;
        write_hash_element(
            writer,
            hash_algo_name,
            &dir_entry.content_hash,
            None,
            Some(&hashdate),
        )?;
        writer.write_event(Event::End(BytesEnd::new("content")))?;

        // <structure>
        writer.write_event(Event::Start(BytesStart::new("structure")))?;
        write_hash_element(
            writer,
            hash_algo_name,
            &dir_entry.structure_hash,
            None,
            Some(&hashdate),
        )?;
        writer.write_event(Event::End(BytesEnd::new("structure")))?;

        writer.write_event(Event::End(BytesEnd::new("directoryhash")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("hashes")))?;
    Ok(())
}

/// Write a single <hash> entry for a file
fn write_hash_entry(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    entry: &MhlHashEntry,
) -> Result<()> {
    writer.write_event(Event::Start(BytesStart::new("hash")))?;

    // <path size="..." lastmodificationdate="...">relative/path</path>
    let mut path_elem = BytesStart::new("path");
    path_elem.push_attribute(("size", entry.file_size.to_string().as_str()));
    path_elem.push_attribute((
        "lastmodificationdate",
        entry.last_modified.to_rfc3339().as_str(),
    ));
    writer.write_event(Event::Start(path_elem))?;
    writer.write_event(Event::Text(BytesText::new(&entry.path)))?;
    writer.write_event(Event::End(BytesEnd::new("path")))?;

    // Hash algorithm elements (one per algorithm)
    for hash in &entry.hashes {
        let algo_name = algorithm_to_xml_name(&hash.algorithm);
        write_hash_element(
            writer,
            algo_name,
            &hash.hex_digest,
            Some(&hash.action.to_string()),
            Some(&hash.hash_date.to_rfc3339()),
        )?;
    }

    writer.write_event(Event::End(BytesEnd::new("hash")))?;
    Ok(())
}

/// Write a hash algorithm element like <xxh64 action="original" hashdate="...">hexvalue</xxh64>
fn write_hash_element(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    element_name: &str,
    hex_digest: &str,
    action: Option<&str>,
    hashdate: Option<&str>,
) -> Result<()> {
    let mut elem = BytesStart::new(element_name);
    if let Some(action_val) = action {
        elem.push_attribute(("action", action_val));
    }
    if let Some(date_val) = hashdate {
        elem.push_attribute(("hashdate", date_val));
    }
    writer.write_event(Event::Start(elem))?;
    writer.write_event(Event::Text(BytesText::new(hex_digest)))?;
    writer.write_event(Event::End(BytesEnd::new(element_name)))?;
    Ok(())
}

/// Helper: write a simple <tag>text</tag> element
fn write_text_element(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    tag: &str,
    text: &str,
) -> Result<()> {
    writer.write_event(Event::Start(BytesStart::new(tag)))?;
    writer.write_event(Event::Text(BytesText::new(text)))?;
    writer.write_event(Event::End(BytesEnd::new(tag)))?;
    Ok(())
}

// ─── Chain File I/O ───────────────────────────────────────────────────────────

/// Write the ascmhl_chain.xml file
pub async fn write_chain_file(path: &Path, chain: &[MhlChainEntry]) -> Result<()> {
    let xml_bytes = generate_chain_xml(chain)?;
    tokio::fs::write(path, &xml_bytes)
        .await
        .with_context(|| format!("Failed to write chain file: {:?}", path))?;
    Ok(())
}

/// Generate chain XML bytes in memory
fn generate_chain_xml(chain: &[MhlChainEntry]) -> Result<Vec<u8>> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

    // XML declaration
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    // <ascmhldirectory xmlns="urn:ASC:MHL:DIRECTORY:v2.0">
    let mut root = BytesStart::new("ascmhldirectory");
    root.push_attribute(("xmlns", CHAIN_NAMESPACE));
    writer.write_event(Event::Start(root))?;

    for entry in chain {
        // <hashlist sequencenr="N">
        let mut hashlist = BytesStart::new("hashlist");
        hashlist.push_attribute(("sequencenr", entry.sequence_nr.to_string().as_str()));
        writer.write_event(Event::Start(hashlist))?;

        // <path>filename.mhl</path>
        write_text_element(&mut writer, "path", &entry.path)?;

        // <sha256>hex_hash</sha256>
        // (ASC MHL spec uses C4 here; we use SHA-256 for the MVP)
        write_text_element(&mut writer, "sha256", &entry.reference_hash)?;

        // </hashlist>
        writer.write_event(Event::End(BytesEnd::new("hashlist")))?;
    }

    // </ascmhldirectory>
    writer.write_event(Event::End(BytesEnd::new("ascmhldirectory")))?;

    let result = writer.into_inner().into_inner();
    Ok(result)
}

/// Parse an existing ascmhl_chain.xml file and return chain entries
pub async fn parse_chain_file(path: &Path) -> Result<Vec<MhlChainEntry>> {
    let content = tokio::fs::read(path)
        .await
        .with_context(|| format!("Failed to read chain file: {:?}", path))?;

    parse_chain_xml(&content)
}

/// Parse chain XML bytes into chain entries
fn parse_chain_xml(xml_bytes: &[u8]) -> Result<Vec<MhlChainEntry>> {
    use quick_xml::Reader;

    let mut reader = Reader::from_reader(xml_bytes);
    reader.trim_text(true);

    let mut entries: Vec<MhlChainEntry> = Vec::new();
    let mut current_entry: Option<MhlChainEntry> = None;
    let mut current_element = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match local_name.as_str() {
                    "hashlist" => {
                        let mut seq_nr: u32 = 0;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"sequencenr" {
                                seq_nr = String::from_utf8_lossy(&attr.value)
                                    .parse()
                                    .unwrap_or(0);
                            }
                        }
                        current_entry = Some(MhlChainEntry {
                            sequence_nr: seq_nr,
                            path: String::new(),
                            reference_hash: String::new(),
                        });
                    }
                    "path" | "sha256" | "c4" => {
                        current_element = local_name;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if let Some(ref mut entry) = current_entry {
                    match current_element.as_str() {
                        "path" => entry.path = text,
                        "sha256" | "c4" => entry.reference_hash = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if local_name == "hashlist" {
                    if let Some(entry) = current_entry.take() {
                        entries.push(entry);
                    }
                }
                current_element.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                anyhow::bail!("Error parsing chain XML at position {}: {:?}", reader.buffer_position(), e);
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash_engine::HashAlgorithm;
    use crate::mhl::{
        MhlAuthor, MhlCreatorInfo, MhlFileHash, MhlGeneration, MhlHashAction, MhlHashEntry,
        MhlProcessType,
    };
    use chrono::Utc;

    fn sample_generation() -> MhlGeneration {
        let now = chrono::DateTime::parse_from_rfc3339("2024-06-15T10:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);

        MhlGeneration {
            generation: 1,
            creation_date: now,
            creator_info: MhlCreatorInfo {
                tool_name: "DIT System".to_string(),
                tool_version: "0.1.0".to_string(),
                hostname: Some("MacStudio.local".to_string()),
                location: Some("Stage 5, Burbank".to_string()),
                comment: Some("Card A offload".to_string()),
                authors: vec![MhlAuthor {
                    name: "John Smith".to_string(),
                    email: Some("john@example.com".to_string()),
                    phone: None,
                    role: Some("DIT".to_string()),
                }],
            },
            process_type: MhlProcessType::Transfer,
            root_content_hash: Some("abc123def456".to_string()),
            root_structure_hash: Some("789xyz000111".to_string()),
            ignore_patterns: vec![
                ".DS_Store".to_string(),
                "ascmhl".to_string(),
                "ascmhl/".to_string(),
            ],
            hash_entries: vec![
                MhlHashEntry {
                    path: "Clips/A002C006.mov".to_string(),
                    file_size: 1073741824,
                    last_modified: now,
                    hashes: vec![MhlFileHash {
                        algorithm: HashAlgorithm::XXH64,
                        hex_digest: "0ea03b369a463d9d".to_string(),
                        action: MhlHashAction::Original,
                        hash_date: now,
                    }],
                },
                MhlHashEntry {
                    path: "Clips/A002C007.mov".to_string(),
                    file_size: 536870912,
                    last_modified: now,
                    hashes: vec![
                        MhlFileHash {
                            algorithm: HashAlgorithm::XXH64,
                            hex_digest: "7680e5f98f4a80fd".to_string(),
                            action: MhlHashAction::Original,
                            hash_date: now,
                        },
                        MhlFileHash {
                            algorithm: HashAlgorithm::SHA256,
                            hex_digest: "a1b2c3d4e5f6".to_string(),
                            action: MhlHashAction::Original,
                            hash_date: now,
                        },
                    ],
                },
            ],
            directory_hashes: Vec::new(),
            hash_algorithm: HashAlgorithm::XXH64,
        }
    }

    #[test]
    fn test_generate_manifest_xml_structure() {
        let gen = sample_generation();
        let xml_bytes = generate_manifest_xml(&gen).unwrap();
        let xml_str = String::from_utf8(xml_bytes).unwrap();

        // Verify XML declaration
        assert!(xml_str.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));

        // Verify namespace
        assert!(xml_str.contains("xmlns=\"urn:ASC:MHL:v2.0\""));
        assert!(xml_str.contains("version=\"2.0\""));

        // Verify creatorinfo
        assert!(xml_str.contains("<creatorinfo>"));
        assert!(xml_str.contains("<creationdate>"));
        assert!(xml_str.contains("<hostname>MacStudio.local</hostname>"));
        assert!(xml_str.contains("<tool version=\"0.1.0\">DIT System</tool>"));
        assert!(xml_str.contains("<location>Stage 5, Burbank</location>"));
        assert!(xml_str.contains("<comment>Card A offload</comment>"));

        // Verify author
        assert!(xml_str.contains("<author>"));
        assert!(xml_str.contains("<name>John Smith</name>"));
        assert!(xml_str.contains("<email>john@example.com</email>"));
        assert!(xml_str.contains("<role>DIT</role>"));

        // Verify processinfo
        assert!(xml_str.contains("<processinfo>"));
        assert!(xml_str.contains("<process>transfer</process>"));
        assert!(xml_str.contains("<roothash>"));

        // Verify ignore patterns
        assert!(xml_str.contains("<ignore>"));
        assert!(xml_str.contains("<pattern>.DS_Store</pattern>"));
        assert!(xml_str.contains("<pattern>ascmhl</pattern>"));

        // Verify hash entries
        assert!(xml_str.contains("<hashes>"));
        assert!(xml_str.contains("Clips/A002C006.mov"));
        assert!(xml_str.contains("0ea03b369a463d9d"));
        assert!(xml_str.contains("action=\"original\""));
        assert!(xml_str.contains("size=\"1073741824\""));

        // Verify multi-algorithm entry
        assert!(xml_str.contains("Clips/A002C007.mov"));
        assert!(xml_str.contains("<xxh64"));
        assert!(xml_str.contains("<sha256"));
    }

    #[test]
    fn test_generate_chain_xml() {
        let chain = vec![
            MhlChainEntry {
                sequence_nr: 1,
                path: "0001_MediaRoot_2024-06-15_103000Z.mhl".to_string(),
                reference_hash: "abc123".to_string(),
            },
            MhlChainEntry {
                sequence_nr: 2,
                path: "0002_MediaRoot_2024-06-16_090000Z.mhl".to_string(),
                reference_hash: "def456".to_string(),
            },
        ];

        let xml_bytes = generate_chain_xml(&chain).unwrap();
        let xml_str = String::from_utf8(xml_bytes).unwrap();

        assert!(xml_str.contains("xmlns=\"urn:ASC:MHL:DIRECTORY:v2.0\""));
        assert!(xml_str.contains("sequencenr=\"1\""));
        assert!(xml_str.contains("sequencenr=\"2\""));
        assert!(xml_str.contains("0001_MediaRoot_2024-06-15_103000Z.mhl"));
        assert!(xml_str.contains("0002_MediaRoot_2024-06-16_090000Z.mhl"));
        assert!(xml_str.contains("<sha256>abc123</sha256>"));
        assert!(xml_str.contains("<sha256>def456</sha256>"));
    }

    #[test]
    fn test_chain_xml_roundtrip() {
        let original_chain = vec![
            MhlChainEntry {
                sequence_nr: 1,
                path: "0001_Card_A_2024-01-15_120000Z.mhl".to_string(),
                reference_hash: "aabbccdd11223344".to_string(),
            },
            MhlChainEntry {
                sequence_nr: 2,
                path: "0002_Card_A_2024-01-16_093000Z.mhl".to_string(),
                reference_hash: "eeff00112233aabb".to_string(),
            },
            MhlChainEntry {
                sequence_nr: 3,
                path: "0003_Card_A_2024-01-17_150000Z.mhl".to_string(),
                reference_hash: "deadbeefcafebabe".to_string(),
            },
        ];

        // Write → Parse → Compare
        let xml_bytes = generate_chain_xml(&original_chain).unwrap();
        let parsed = parse_chain_xml(&xml_bytes).unwrap();

        assert_eq!(parsed.len(), 3);
        for (orig, parsed) in original_chain.iter().zip(parsed.iter()) {
            assert_eq!(orig.sequence_nr, parsed.sequence_nr);
            assert_eq!(orig.path, parsed.path);
            assert_eq!(orig.reference_hash, parsed.reference_hash);
        }
    }

    #[test]
    fn test_parse_empty_chain() {
        let chain: Vec<MhlChainEntry> = Vec::new();
        let xml_bytes = generate_chain_xml(&chain).unwrap();
        let parsed = parse_chain_xml(&xml_bytes).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_algorithm_xml_names() {
        assert_eq!(algorithm_to_xml_name(&HashAlgorithm::XXH64), "xxh64");
        assert_eq!(algorithm_to_xml_name(&HashAlgorithm::XXH3), "xxh3");
        assert_eq!(algorithm_to_xml_name(&HashAlgorithm::XXH128), "xxh128");
        assert_eq!(algorithm_to_xml_name(&HashAlgorithm::SHA256), "sha256");
        assert_eq!(algorithm_to_xml_name(&HashAlgorithm::MD5), "md5");
    }

    #[tokio::test]
    async fn test_write_and_read_chain_file() {
        let dir = tempfile::tempdir().unwrap();
        let chain_path = dir.path().join("ascmhl_chain.xml");

        let chain = vec![MhlChainEntry {
            sequence_nr: 1,
            path: "0001_test_2024-01-01_000000Z.mhl".to_string(),
            reference_hash: "test_hash_value".to_string(),
        }];

        // Write
        write_chain_file(&chain_path, &chain).await.unwrap();
        assert!(chain_path.exists());

        // Read back
        let parsed = parse_chain_file(&chain_path).await.unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].sequence_nr, 1);
        assert_eq!(parsed[0].path, "0001_test_2024-01-01_000000Z.mhl");
        assert_eq!(parsed[0].reference_hash, "test_hash_value");
    }

    #[tokio::test]
    async fn test_write_manifest_file() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("0001_test_2024-06-15_103000Z.mhl");

        let gen = sample_generation();
        write_manifest(&manifest_path, &gen).await.unwrap();

        assert!(manifest_path.exists());

        // Read back and verify it's valid XML
        let content = tokio::fs::read_to_string(&manifest_path).await.unwrap();
        assert!(content.starts_with("<?xml"));
        assert!(content.contains("<hashlist"));
        assert!(content.contains("</hashlist>"));
    }
}
