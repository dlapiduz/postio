//! Parses the corpus with a second, dumber implementation than `postio_model::mime`, so a test
//! comparing the two compares independent readings. Section fetches return bytes as they sit in the
//! file, undecoded; `message/rfc822` is a leaf.

use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Part {
    pub(super) kind: String,
    pub(super) subtype: String,
    pub(super) params: Vec<(String, String)>,
    pub(super) encoding: String,
    pub(super) id: Option<String>,
    pub(super) description: Option<String>,
    pub(super) disposition: Option<(String, Vec<(String, String)>)>,
    pub(super) header: Range<usize>,
    pub(super) body: Range<usize>,
    pub(super) children: Vec<Part>,
}

impl Part {
    fn is_multipart(&self) -> bool {
        self.kind == "MULTIPART"
    }

    fn is_text(&self) -> bool {
        self.kind == "TEXT"
    }
}

pub(super) fn parse(raw: &[u8]) -> Part {
    parse_range(raw, 0..raw.len())
}

fn parse_range(raw: &[u8], range: Range<usize>) -> Part {
    let (header, body) = split_header(raw, range);
    let fields = headers(raw, header.clone());

    let content_type = field(&fields, "content-type").unwrap_or_default();
    let (kind, subtype, params) = content_type_of(&content_type);
    let encoding = field(&fields, "content-transfer-encoding")
        .map(|value| value.trim().to_ascii_uppercase())
        .unwrap_or_else(|| "7BIT".to_owned());
    let disposition = field(&fields, "content-disposition").map(|value| {
        let (kind, _, params) = content_type_of(&value);
        (kind, params)
    });

    let mut part = Part {
        kind,
        subtype,
        params,
        encoding,
        id: field(&fields, "content-id"),
        description: field(&fields, "content-description"),
        disposition,
        header,
        body: body.clone(),
        children: Vec::new(),
    };

    if part.is_multipart()
        && let Some(boundary) = part
            .params
            .iter()
            .find(|(name, _)| name == "BOUNDARY")
            .map(|(_, value)| value.clone())
    {
        part.children = split_parts(raw, body, &boundary)
            .into_iter()
            .map(|child| parse_range(raw, child))
            .collect();
    }

    part
}

/// The blank line belongs to the header so `BODY[HEADER]` ends with one (RFC 3501 §6.4.5).
fn split_header(raw: &[u8], range: Range<usize>) -> (Range<usize>, Range<usize>) {
    let slice = &raw[range.clone()];
    if let Some(at) = find(slice, b"\r\n\r\n") {
        let end = range.start + at + 4;
        return (range.start..end, end..range.end);
    }
    if let Some(at) = find(slice, b"\n\n") {
        let end = range.start + at + 2;
        return (range.start..end, end..range.end);
    }
    (range.clone(), range.end..range.end)
}

fn headers(raw: &[u8], range: Range<usize>) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(&raw[range]);
    let mut fields: Vec<(String, String)> = Vec::new();

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            fields.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }

    fields
}

fn field(fields: &[(String, String)], name: &str) -> Option<String> {
    fields
        .iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.clone())
}

fn content_type_of(value: &str) -> (String, String, Vec<(String, String)>) {
    let mut pieces = value.split(';');
    let head = pieces.next().unwrap_or("").trim();
    let (kind, subtype) = match head.split_once('/') {
        Some((kind, subtype)) => (kind.trim(), subtype.trim()),
        None if head.is_empty() => ("TEXT", "PLAIN"),
        None => (head, ""),
    };

    let params = pieces
        .filter_map(|piece| {
            let (name, value) = piece.split_once('=')?;
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(value);
            Some((name.trim().to_ascii_uppercase(), value.to_owned()))
        })
        .collect();

    (
        kind.to_ascii_uppercase(),
        subtype.to_ascii_uppercase(),
        params,
    )
}

fn split_parts(raw: &[u8], body: Range<usize>, boundary: &str) -> Vec<Range<usize>> {
    let delimiter = format!("--{boundary}");
    let slice = &raw[body.clone()];

    let mut marks: Vec<(usize, bool)> = Vec::new();
    let mut at = 0usize;
    while at < slice.len() {
        let line_end = find(&slice[at..], b"\n").map_or(slice.len(), |end| at + end + 1);
        let line = trim_eol(&slice[at..line_end]);
        if line.starts_with(delimiter.as_bytes()) {
            let closing = line.len() >= delimiter.len() + 2
                && &line[delimiter.len()..delimiter.len() + 2] == b"--";
            marks.push((at, closing));
            if closing {
                break;
            }
        }
        at = line_end;
    }

    let mut parts = Vec::new();
    for window in marks.windows(2) {
        let (start, _) = window[0];
        let (end, _) = window[1];
        let start = find(&slice[start..], b"\n").map_or(end, |at| start + at + 1);
        let end = trim_trailing_eol(&slice[..end]).len().max(start);
        if start <= end {
            parts.push(body.start + start..body.start + end);
        }
    }
    parts
}

pub(super) fn body_structure(raw: &[u8], part: &Part) -> String {
    if part.is_multipart() {
        let children: String = part
            .children
            .iter()
            .map(|child| body_structure(raw, child))
            .collect::<Vec<_>>()
            .join("");
        return format!(
            "({children} {subtype} {params} {disposition} NIL NIL)",
            subtype = string(&part.subtype),
            params = parameters(&part.params),
            disposition = disposition(part),
        );
    }

    let size = part.body.len();
    let mut fields = format!(
        "{kind} {subtype} {params} {id} {description} {encoding} {size}",
        kind = string(&part.kind),
        subtype = string(&part.subtype),
        params = parameters(&part.params),
        id = nstring(part.id.as_deref()),
        description = nstring(part.description.as_deref()),
        encoding = string(&part.encoding),
    );
    if part.is_text() {
        fields.push_str(&format!(" {}", lines(&raw[part.body.clone()])));
    }

    format!("({fields} NIL {} NIL NIL)", disposition(part))
}

fn disposition(part: &Part) -> String {
    match &part.disposition {
        None => "NIL".to_owned(),
        Some((kind, params)) => format!("({} {})", string(kind), parameters(params)),
    }
}

fn parameters(params: &[(String, String)]) -> String {
    let rendered: Vec<String> = params
        .iter()
        .filter(|(name, _)| name != "BOUNDARY")
        .map(|(name, value)| format!("{} {}", string(name), string(value)))
        .collect();
    if rendered.is_empty() {
        "NIL".to_owned()
    } else {
        format!("({})", rendered.join(" "))
    }
}

fn lines(body: &[u8]) -> usize {
    body.iter().filter(|byte| **byte == b'\n').count()
}

pub(super) fn envelope(raw: &[u8], part: &Part) -> String {
    let fields = headers(raw, part.header.clone());
    let get = |name: &str| field(&fields, name);

    // RFC 3501: a missing `Sender` or `Reply-To` defaults to `From`.
    let from = get("from").unwrap_or_default();
    let sender = get("sender").unwrap_or_else(|| from.clone());
    let reply_to = get("reply-to").unwrap_or_else(|| from.clone());

    format!(
        "({date} {subject} {from} {sender} {reply_to} {to} {cc} {bcc} {in_reply_to} {message_id})",
        date = nstring(get("date").as_deref()),
        subject = nstring(get("subject").as_deref()),
        from = addresses(&from),
        sender = addresses(&sender),
        reply_to = addresses(&reply_to),
        to = addresses(&get("to").unwrap_or_default()),
        cc = addresses(&get("cc").unwrap_or_default()),
        bcc = addresses(&get("bcc").unwrap_or_default()),
        in_reply_to = nstring(get("in-reply-to").as_deref()),
        message_id = nstring(get("message-id").as_deref()),
    )
}

fn addresses(value: &str) -> String {
    let rendered: Vec<String> = split_addresses(value)
        .iter()
        .map(|address| {
            let (name, mailbox) = match (address.find('<'), address.rfind('>')) {
                (Some(open), Some(close)) if close > open => {
                    (address[..open].trim(), address[open + 1..close].trim())
                }
                _ => ("", address.trim()),
            };
            let name = name.trim().trim_matches('"').trim();
            let (local, host) = match mailbox.rsplit_once('@') {
                Some((local, host)) => (local, host),
                None => (mailbox, ""),
            };
            format!(
                "({} NIL {} {})",
                nstring((!name.is_empty()).then_some(name)),
                nstring((!local.is_empty()).then_some(local)),
                nstring((!host.is_empty()).then_some(host)),
            )
        })
        .collect();

    if rendered.is_empty() {
        "NIL".to_owned()
    } else {
        format!("({})", rendered.join(""))
    }
}

fn split_addresses(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut angled = false;

    for character in value.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                current.push(character);
            }
            '<' if !quoted => {
                angled = true;
                current.push(character);
            }
            '>' if !quoted => {
                angled = false;
                current.push(character);
            }
            ',' if !quoted && !angled => {
                push_address(&mut out, &current);
                current.clear();
            }
            _ => current.push(character),
        }
    }
    push_address(&mut out, &current);
    out
}

fn push_address(out: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_owned());
    }
}

/// A leaf yields its body; a `multipart/*` node yields headers and body together, as inside its
/// parent.
pub(super) fn section(raw: &[u8], part: &Part, spec: &str) -> Option<Vec<u8>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Some(raw.to_vec());
    }

    let upper = spec.to_ascii_uppercase();
    if upper == "HEADER" {
        return Some(raw[part.header.clone()].to_vec());
    }
    if upper == "TEXT" {
        return Some(raw[part.body.clone()].to_vec());
    }
    if let Some(rest) = upper.strip_prefix("HEADER.FIELDS") {
        let wanted: Vec<String> = rest
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split_whitespace()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        return Some(selected_headers(raw, part, &wanted));
    }

    let (path, suffix) = match upper.rsplit_once('.') {
        Some((path, tail)) if matches!(tail, "MIME" | "HEADER" | "TEXT") => (path, Some(tail)),
        _ => (upper.as_str(), None),
    };

    let target = walk(part, path)?;
    Some(match suffix {
        Some("MIME") | Some("HEADER") => raw[target.header.clone()].to_vec(),
        Some("TEXT") => raw[target.body.clone()].to_vec(),
        _ if target.is_multipart() => raw[target.header.start..target.body.end].to_vec(),
        _ => raw[target.body.clone()].to_vec(),
    })
}

/// In a non-multipart message, part `1` is the message body itself (RFC 3501).
fn walk<'a>(part: &'a Part, path: &str) -> Option<&'a Part> {
    let mut current = part;
    for piece in path.split('.') {
        let index: usize = piece.parse().ok()?;
        if index == 0 {
            return None;
        }
        if current.children.is_empty() {
            return (index == 1).then_some(current);
        }
        current = current.children.get(index - 1)?;
    }
    Some(current)
}

fn selected_headers(raw: &[u8], part: &Part, wanted: &[String]) -> Vec<u8> {
    let text = String::from_utf8_lossy(&raw[part.header.clone()]);
    let mut out = String::new();
    let mut keeping = false;

    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        if bare.is_empty() {
            break;
        }
        if bare.starts_with([' ', '\t']) {
            if keeping {
                out.push_str(line);
            }
            continue;
        }
        keeping = bare
            .split_once(':')
            .map(|(name, _)| {
                wanted
                    .iter()
                    .any(|want| name.trim().eq_ignore_ascii_case(want))
            })
            .unwrap_or(false);
        if keeping {
            out.push_str(line);
        }
    }

    out.push_str("\r\n");
    out.into_bytes()
}

pub(super) fn string(value: &str) -> String {
    if value.is_ascii()
        && !value.contains(['\r', '\n'])
        && value.bytes().all(|byte| byte >= 0x20 && byte != 0x7f)
    {
        return format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
    }
    format!("{{{}}}\r\n{value}", value.len())
}

pub(super) fn nstring(value: Option<&str>) -> String {
    match value {
        None => "NIL".to_owned(),
        Some(value) => string(value),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn trim_eol(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    &line[..end]
}

fn trim_trailing_eol(slice: &[u8]) -> &[u8] {
    trim_eol(slice)
}
