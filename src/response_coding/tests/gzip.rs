use std::io::Write;

use super::*;

fn member_with_optional_fields(body: &[u8]) -> Vec<u8> {
    let mut header = vec![0x1f, 0x8b, 8, 0x1e, 0, 0, 0, 0, 0, 255];
    header.extend_from_slice(&[3, 0, 1, 2, 3]);
    header.extend_from_slice(b"fixture-name\0fixture-comment\0");
    let mut crc = flate2::Crc::new();
    crc.update(&header);
    header.extend_from_slice(&(crc.sum() as u16).to_le_bytes());
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).unwrap();
    header.extend(encoder.finish().unwrap());
    crc.reset();
    crc.update(body);
    header.extend_from_slice(&crc.sum().to_le_bytes());
    header.extend_from_slice(&crc.amount().to_le_bytes());
    header
}

#[tokio::test]
async fn optional_header_fields_and_concatenated_members_decode_at_byte_splits() {
    let mut bytes = member_with_optional_fields(b"first member");
    bytes.extend(member_with_optional_fields(b"second member"));
    let input = stream::iter(
        bytes
            .into_iter()
            .map(|byte| Ok::<_, Infallible>(Bytes::from(vec![byte]))),
    );
    assert_eq!(
        collected(decode(input, UpstreamCoding::Gzip))
            .await
            .unwrap(),
        b"first membersecond member"
    );
}

#[tokio::test]
async fn gzip_rejects_header_crc_body_crc_size_and_truncated_member() {
    let bytes = member_with_optional_fields(b"body");
    let header_crc = 10 + 5 + b"fixture-name\0fixture-comment\0".len();
    for offset in [0, 2, 3, header_crc, bytes.len() - 8, bytes.len() - 4] {
        let mut bad = bytes.clone();
        bad[offset] ^= 0xff;
        let input = stream::iter([Ok::<_, Infallible>(Bytes::from(bad))]);
        assert!(
            collected(decode(input, UpstreamCoding::Gzip))
                .await
                .is_err(),
            "offset {offset}"
        );
    }
    for len in 0..bytes.len() {
        let input = stream::iter([Ok::<_, Infallible>(Bytes::copy_from_slice(&bytes[..len]))]);
        assert!(
            collected(decode(input, UpstreamCoding::Gzip))
                .await
                .is_err(),
            "length {len}"
        );
    }
}

#[tokio::test]
async fn optional_text_header_is_scanned_without_a_header_sized_buffer() {
    let body = b"bounded output";
    let member = member_with_optional_fields(body);
    let mut bytes = vec![0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 0, 255];
    bytes.extend(std::iter::repeat_n(b'n', 128 * 1024));
    bytes.push(0);
    let start = 10 + 5 + b"fixture-name\0fixture-comment\0".len() + 2;
    bytes.extend_from_slice(&member[start..]);
    let input = stream::iter([Ok::<_, Infallible>(Bytes::from(bytes))]);
    assert_eq!(
        collected(decode(input, UpstreamCoding::Gzip))
            .await
            .unwrap(),
        body
    );
}
