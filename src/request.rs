//! HTTP request support.

extern crate time;

use std::ascii::AsciiExt;
use std::iter::FromIterator;

use super::error::*;
use super::con::Connection;  // interesting, wildcard doesn't work for this.
use super::ascii::AsciiPrefix;

/// Accepts a request from the given `Connection` and returns its contents, or
/// an error.
///
/// Errors may be returned *during* reception of the request.  While a
/// `Connection` can theoretically be kept open after an error, I haven't done
/// the legwork on this yet.
pub fn read(c: &mut Connection) -> Result<Request> {
  // Take the first non-blank line as the Request-Line (5.1).
  // Our tolerance of multiple blank lines between requests on a connection, and
  // blank lines before the initial request, mimics Publicfile, but does not
  // appear to be required or suggested by the standard.
  let request_line = {
    let mut line;
    loop {
      line = try!(c.readline());
      // Tolerate and skip blank lines between requests.
      if !line.is_empty() { break }
    }
    line
  };

  let mut req = try!(parse_request_line(request_line));

  // Collect headers from the connection.  There is some overlap between the
  // information in headers and the information conveyed in the request-line,
  // so we load it into the request as we find it.

  let mut hdr = Vec::new();  // Accumulates lines of header.

  loop {
    let hdr_line = try!(c.readline());

    // Requests headers are slightly complicated because they can be broken
    // over multiple lines using indentation.
    if !hdr.is_empty() && (hdr_line.is_empty() || !is_http_ws(hdr_line[0])) {
      // At an empty line or a line beginning with non-whitespace, we know we
      // have received the entirety of the *previous* header and can process
      // it.  Only bother if we've accumulated some header; otherwise we're
      // dealing with the empty terminating line.
      if hdr.starts_with_ignore_ascii_case(b"content-length:")
          || hdr.starts_with_ignore_ascii_case(b"transfer-encoding:") {
        return Err(HttpError::NotImplemented(b"I can't receive messages"))
      }
      if hdr.starts_with_ignore_ascii_case(b"expect") {
        return Err(HttpError::SpanishInquisition)
      }
      if hdr.starts_with_ignore_ascii_case(b"if-match")
          || hdr.starts_with_ignore_ascii_case(b"if-unmodified-since") {
        return Err(HttpError::PreconditionFailed)
      }

      if hdr.starts_with_ignore_ascii_case(b"host") {
        // Only accept a host from the request headers if none was provided
        // in the start line.
        if req.host.is_none() {
          // Just drop whitespace characters from the host header.  This
          // questionable interpretation of the spec mimics publicfile.
          let new_host = Vec::from_iter(hdr[5..].iter().cloned()
              .filter(|b| !is_http_ws(*b)));
          if !new_host.is_empty() { req.host = Some(new_host) }
        }
      }

      if hdr.starts_with_ignore_ascii_case(b"if-modified-since") {
        req.if_modified_since =
            Some(Vec::from_iter(hdr[18..].iter().cloned()
                                  .skip_while(|b| is_http_ws(*b))));
      }

      // We've processed this header -- discard it.
      hdr.clear();
    }

    if hdr_line.is_empty() { break }

    hdr.extend(hdr_line);
  }

  Ok(req)
}

fn is_http_ws(c: u8) -> bool {
  c == b' ' || c == b'\t'
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Method {
  Get,
  Head,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Protocol {
  Http10,
  Http11,
}

fn indexof<T: PartialEq>(slice: &[T], item: T) -> usize {
  for i in 0..slice.len() {
    if slice[i] == item { return i }
  }
  slice.len()
}

fn parse_request_line(line: Vec<u8>) -> Result<Request> {
  let parts: Vec<_> = line.splitn(3, |b| *b == b' ').collect();
  if parts.len() != 3 { return Err(HttpError::BadRequest) }

  let method = match parts[0] {
    b"GET" => Method::Get,
    b"HEAD" => Method::Head,
    _ => return Err(HttpError::BadMethod),
  };
  let (host, mut path) = {
    let raw = parts[1];
    // Distinguish an old-style path-only request from a HTTP/1.1-style URL
    // request by checking for the presence of an HTTP scheme.
    if raw.len() >= 7 && raw[..7].eq_ignore_ascii_case(b"http://") {
      // Split the remainder at the first slash.  The bytes to the left are the
      // host name; to the right, including the delimiter, the path.
      let (host, path) = raw[7..].split_at(indexof(&raw[7..], b'/'));
      let host = Vec::from_iter(host.into_iter().cloned());
      let path = Vec::from_iter(path.into_iter().cloned());

      if host.is_empty() {
        // The client can totally specify an "empty host" using a URL of the
        // form `http:///foo`.  We are not amused, and treat this as an absent
        // host specification.
        (None, path)
      } else {
        (Some(host), path) 
      }
    } else {
      (None, Vec::from_iter(parts[1].into_iter().cloned()))
    }
  };
  let protocol = match parts[2] {
    b"HTTP/1.0" => Protocol::Http10,
    b"HTTP/1.1" => Protocol::Http11,
    _ => return Err(HttpError::BadProtocol),
  };

  // Slap an 'index.html' onto the end of any path that, from simple textual
  // inspection, ends in a directory.
  if path.is_empty() || path.ends_with(b"/") {
    path.extend(b"index.html".into_iter().cloned());
  }

  Ok(Request {
    method: method,
    protocol: protocol,
    host: host,
    path: path,
    if_modified_since: None,  // Filled in later.
  }) 
}

#[derive(Debug)]
pub struct Request {
  pub method: Method,
  pub protocol: Protocol,
  pub host: Option<Vec<u8>>,
  pub path: Vec<u8>,
  pub if_modified_since: Option<Vec<u8>>,
}


