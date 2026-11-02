use crate::ui::locale::tr;
use anyhow::{Context, Result, ensure};
use std::io::{BufRead, BufReader, Read, Take};

const MAX_RESPONSE: u64 = 1024 * 1024;

// A login uses one TLS connection. Respect its HTTP framing even when the
// server leaves the socket open, and bound headers, chunks and trailers together.
pub(super) fn read_response(stream: impl Read) -> Result<(u16, Vec<u8>)> {
    let mut input = BufReader::new(stream).take(MAX_RESPONSE + 1);
    let head = header_block(&mut input)?;
    let mut fields = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut fields);
    ensure!(
        response.parse(&head)?.is_complete(),
        tr("error.http_incomplete")
    );
    let status = response.code.context(tr("error.http_status"))?;

    let mut length = None;
    let mut chunked = false;
    for field in response.headers.iter() {
        if field.name.eq_ignore_ascii_case("Content-Length") {
            ensure!(
                field.value.iter().all(u8::is_ascii_digit),
                tr("error.http_framing")
            );
            let value = std::str::from_utf8(field.value)?.parse::<u64>()?;
            ensure!(length.is_none_or(|n| n == value), tr("error.http_framing"));
            length = Some(value);
        } else if field.name.eq_ignore_ascii_case("Transfer-Encoding") {
            ensure!(
                !chunked && field.value.eq_ignore_ascii_case(b"chunked"),
                tr("error.http_framing")
            );
            chunked = true;
        }
    }
    let mut body = Vec::new();
    match (chunked, length) {
        (true, Some(_)) => anyhow::bail!(tr("error.http_framing")),
        (true, None) => {
            let mut line = Vec::new();
            loop {
                line.clear();
                ensure!(
                    input.read_until(b'\n', &mut line)? > 0,
                    tr("error.chunk_incomplete")
                );
                ensure!(line[0].is_ascii_hexdigit(), tr("error.chunk_invalid"));
                let httparse::Status::Complete((_, size)) = httparse::parse_chunk_size(&line)
                    .map_err(|_| anyhow::anyhow!(tr("error.chunk_invalid")))?
                else {
                    anyhow::bail!(tr("error.chunk_incomplete"));
                };
                if size == 0 {
                    header_block(&mut input)?;
                    break;
                }
                append_exact(&mut input, &mut body, size)?;
                let mut end = [0; 2];
                input.read_exact(&mut end)?;
                ensure!(&end == b"\r\n", tr("error.chunk_invalid"));
            }
        }
        (false, Some(size)) => append_exact(&mut input, &mut body, size)?,
        (false, None) => {
            input.read_to_end(&mut body)?;
        }
    }
    ensure!(input.limit() > 0, tr("error.response_large"));
    Ok((status, body))
}

fn header_block(input: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut block = Vec::new();
    loop {
        let start = block.len();
        ensure!(
            input.read_until(b'\n', &mut block)? > 0,
            tr("error.http_incomplete")
        );
        if matches!(&block[start..], b"\r\n" | b"\n") {
            return Ok(block);
        }
    }
}

fn append_exact(input: &mut Take<impl Read>, body: &mut Vec<u8>, count: u64) -> Result<()> {
    ensure!(count < input.limit(), tr("error.response_large"));
    let start = body.len();
    body.resize(start + count as usize, 0);
    input
        .read_exact(&mut body[start..])
        .context(tr("error.http_incomplete"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_replies_end_at_the_declared_boundary() {
        for (reply, expected) in [
            (&b"HTTP/1.1 403 Forbidden\r\nContent-Length: 2\r\n\r\n{}extra"[..], (403, &b"{}"[..])),
            (&b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;x=y\r\nte\r\n2\r\nst\r\n0\r\nX-End: 1\r\n\r\nextra"[..], (200, &b"test"[..])),
            (&b"HTTP/1.0 200 OK\r\n\r\nhello"[..], (200, &b"hello"[..])),
        ] {
            let (status, body) = read_response(reply).unwrap();
            assert_eq!((status, body.as_slice()), expected);
        }
    }

    #[test]
    fn incomplete_oversized_and_ambiguous_replies_fail() {
        for tail in [
            "Content-Length: 9\r\n\r\n{}",
            "Content-Length: 18446744073709551615\r\n\r\n",
            "Content-Length: 3\r\nContent-Length: 4\r\n\r\nbody",
            "Transfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n0\r\n\r\n",
            "Transfer-Encoding: chunked\r\n\r\n5\r\na",
            "Transfer-Encoding: chunked\r\n\r\n0\r\n",
            "Transfer-Encoding: chunked\r\n\r\n\r\n\r\n",
        ] {
            assert!(read_response(format!("HTTP/1.1 200 OK\r\n{tail}").as_bytes()).is_err());
        }
    }
}
