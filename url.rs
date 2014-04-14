// Copyright 2013-2014 Simon Sapin.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![crate_id = "rusturl#0.1"]
#![crate_type = "dylib"]
#![crate_type = "rlib"]
#![feature(macro_rules)]

extern crate encoding;

#[cfg(test)]
extern crate serialize;

use std::str;
use std::cmp;

use std::num::ToStrRadix;

use encoding::Encoding;
use encoding::all::UTF_8;


mod parser;
pub mod form_urlencoded;
pub mod punycode;

#[cfg(test)]
mod tests;


#[deriving(Clone)]
pub struct Url {
    pub scheme: ~str,
    pub scheme_data: SchemeData,
    pub query: Option<~str>,  // See form_urlencoded::parse_str() to get name/value pairs.
    pub fragment: Option<~str>,
}

#[deriving(Clone)]
pub enum SchemeData {
    RelativeSchemeData(SchemeRelativeUrl),
    OtherSchemeData(~str),  // data: URLs, mailto: URLs, etc.
}

#[deriving(Clone)]
pub struct SchemeRelativeUrl {
    pub userinfo: Option<UserInfo>,
    pub host: Host,
    pub port: ~str,
    pub path: ~[~str],
}

#[deriving(Clone)]
pub struct UserInfo {
    pub username: ~str,
    pub password: Option<~str>,
}

#[deriving(Clone)]
pub enum Host {
    Domain(~[~str]),  // Can only be empty in the file scheme
    Ipv6(Ipv6Address)
}

pub struct Ipv6Address {
    pub pieces: [u16, ..8]
}

impl Clone for Ipv6Address {
    fn clone(&self) -> Ipv6Address {
        Ipv6Address { pieces: self.pieces }
    }
}


macro_rules! is_match(
    ($value:expr, $($pattern:pat)|+) => (
        match $value { $($pattern)|+ => true, _ => false }
    );
)


pub type ParseResult<T> = Result<T, &'static str>;


impl Url {
    pub fn parse(input: &str, base_url: Option<&Url>) -> ParseResult<Url> {
        parser::parse_url(input, base_url)
    }

    pub fn serialize(&self) -> ~str {
        let mut result = self.serialize_no_fragment();
        match self.fragment {
            None => (),
            Some(ref fragment) => {
                result.push_str("#");
                result.push_str(fragment.as_slice());
            }
        }
        result
    }

    pub fn serialize_no_fragment(&self) -> ~str {
        let mut result = self.scheme.to_owned();
        result.push_str(":");
        match self.scheme_data {
            RelativeSchemeData(SchemeRelativeUrl {
                ref userinfo, ref host, ref port, ref path
            }) => {
                result.push_str("//");
                match userinfo {
                    &None => (),
                    &Some(UserInfo { ref username, ref password })
                    => if username.len() > 0 || password.is_some() {
                        result.push_str(username.as_slice());
                        match password {
                            &None => (),
                            &Some(ref password) => {
                                result.push_str(":");
                                result.push_str(password.as_slice());
                            }
                        }
                        result.push_str("@");
                    }
                }
                result.push_str(host.serialize());
                if port.len() > 0 {
                    result.push_str(":");
                    result.push_str(port.as_slice());
                }
                if path.len() > 0 {
                    for path_part in path.iter() {
                        result.push_str("/");
                        result.push_str(path_part.as_slice());
                    }
                } else {
                    result.push_str("/");
                }
            },
            OtherSchemeData(ref data) => result.push_str(data.as_slice()),
        }
        match self.query {
            None => (),
            Some(ref query) => {
                result.push_str("?");
                result.push_str(query.as_slice());
            }
        }
        result
    }
}


impl Host {
    pub fn parse(input: &str) -> ParseResult<Host> {
        if input.len() == 0 {
            Err("Empty host")
        } else if input[0] == '[' as u8 {
            if input[input.len() - 1] == ']' as u8 {
                Ipv6Address::parse(input.slice(1, input.len() - 1)).map(Ipv6)
            } else {
                Err("Invalid Ipv6 address")
            }
        } else {
            let mut percent_encoded = ~"";
            utf8_percent_encode(input, SimpleEncodeSet, &mut percent_encoded);
            let bytes = percent_decode(percent_encoded.as_bytes());
            let decoded = UTF_8.decode(bytes, encoding::DecodeReplace).unwrap();
            let mut labels = ~[];
            for label in decoded.split(&['.', '\u3002', '\uFF0E', '\uFF61']) {
                // TODO: Remove this check and use IDNA "domain to ASCII"
                // TODO: switch to .map(domain_label_to_ascii).collect() then.
                if label.is_ascii() {
                    labels.push(label.to_owned())
                } else {
                    return Err("Non-ASCII domains (IDNA) are not supported yet.")
                }
            }
            Ok(Domain(labels))
        }
    }

    pub fn serialize(&self) -> ~str {
        match *self {
            Domain(ref labels) => labels.connect("."),
            Ipv6(ref address) => {
                let mut result = ~"[";
                result.push_str(address.serialize());
                result.push_str("]");
                result
            }
        }
    }
}


impl Ipv6Address {
    pub fn parse(input: &str) -> ParseResult<Ipv6Address> {
        let len = input.len();
        let mut is_ip_v4 = false;
        let mut pieces = [0, 0, 0, 0, 0, 0, 0, 0];
        let mut piece_pointer = 0u;
        let mut compress_pointer = None;
        let mut i = 0u;
        if input[0] == ':' as u8 {
            if input[1] != ':' as u8 {
                return Err("Invalid IPv6 address")
            }
            i = 2;
            piece_pointer = 1;
            compress_pointer = Some(1u);
        }

        while i < len {
            if piece_pointer == 8 {
                return Err("Invalid IPv6 address")
            }
            if input[i] == ':' as u8 {
                if compress_pointer.is_some() {
                    return Err("Invalid IPv6 address")
                }
                i += 1;
                piece_pointer += 1;
                compress_pointer = Some(piece_pointer);
                continue
            }
            let start = i;
            let end = cmp::min(len, start + 4);
            let mut value = 0u16;
            while i < end {
                match from_hex(input[i]) {
                    Some(digit) => {
                        value = value * 0x10 + digit as u16;
                        i += 1;
                    },
                    None => break
                }
            }
            if i < len {
                match input[i] as char {
                    '.' => {
                        if i == start {
                            return Err("Invalid IPv6 address")
                        }
                        i = start;
                        is_ip_v4 = true;
                    },
                    ':' => {
                        i += 1;
                        if i == len {
                            return Err("Invalid IPv6 address")
                        }
                    },
                    _ => return Err("Invalid IPv6 address")
                }
            }
            if is_ip_v4 {
                break
            }
            pieces[piece_pointer] = value;
            piece_pointer += 1;
        }

        if is_ip_v4 {
            if piece_pointer > 6 {
                return Err("Invalid IPv6 address")
            }
            let mut dots_seen = 0u;
            while i < len {
                let mut value = 0u16;
                while i < len {
                    let digit = match input[i] {
                        c @ 0x30 .. 0x39 => c - 0x30,  // 0..9
                        _ => break
                    };
                    value = value * 10 + digit as u16;
                    if value > 255 {
                        return Err("Invalid IPv6 address")
                    }
                }
                if dots_seen < 3 && !(i < len && input[i] == '.' as u8) {
                    return Err("Invalid IPv6 address")
                }
                pieces[piece_pointer] = pieces[piece_pointer] * 0x100 + value;
                if dots_seen == 0 || dots_seen == 2 {
                    piece_pointer += 1;
                }
                i += 1;
                if dots_seen == 3 && i < len {
                    return Err("Invalid IPv6 address")
                }
                dots_seen += 1;
            }
        }

        match compress_pointer {
            Some(compress_pointer) => {
                let mut swaps = piece_pointer - compress_pointer;
                piece_pointer = 7;
                while swaps > 0 {
                    pieces[piece_pointer] = pieces[compress_pointer + swaps - 1];
                    pieces[compress_pointer + swaps - 1] = 0;
                    swaps -= 1;
                    piece_pointer -= 1;
                }
            }
            _ => if piece_pointer != 8 {
                return Err("Invalid IPv6 address")
            }
        }
        Ok(Ipv6Address { pieces: pieces })
    }

    pub fn serialize(&self) -> ~str {
        let mut output = ~"";
        let (compress_start, compress_end) = longest_zero_sequence(&self.pieces);
        let mut i = 0;
        while i < 8 {
            if i == compress_start {
                output.push_str(":");
                if i == 0 {
                    output.push_str(":");
                }
                if compress_end < 8 {
                    i = compress_end;
                } else {
                    break;
                }
            }
            output.push_str(self.pieces[i as uint].to_str_radix(16));
            if i < 7 {
                output.push_str(":");
            }
            i += 1;
        }
        output
    }
}


fn longest_zero_sequence(pieces: &[u16, ..8]) -> (int, int) {
    let mut longest = -1;
    let mut longest_length = -1;
    let mut start = -1;
    macro_rules! finish_sequence(
        ($end: expr) => {
            if start >= 0 {
                let length = $end - start;
                if length > longest_length {
                    longest = start;
                    longest_length = length;
                }
            }
        };
    );
    for i in range(0, 8) {
        if pieces[i as uint] == 0 {
            if start < 0 {
                start = i;
            }
        } else {
            finish_sequence!(i);
            start = -1;
        }
    }
    finish_sequence!(8);
    (longest, longest + longest_length)
}


#[inline]
fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        0x30 .. 0x39 => Some(byte - 0x30),  // 0..9
        0x41 .. 0x46 => Some(byte + 10 - 0x41),  // A..F
        0x61 .. 0x66 => Some(byte + 10 - 0x61),  // a..f
        _ => None
    }
}

#[inline]
fn to_hex_upper(value: u8) -> u8 {
    match value {
        0 .. 9 => value + 0x30,
        10 .. 15 => value - 10 + 0x41,
        _ => fail!()
    }
}


enum EncodeSet {
    SimpleEncodeSet,
    DefaultEncodeSet,
    UserInfoEncodeSet,
    PasswordEncodeSet,
    UsernameEncodeSet
}


#[inline]
fn utf8_percent_encode(input: &str, encode_set: EncodeSet, output: &mut ~str) {
    use Default = self::DefaultEncodeSet;
    use UserInfo = self::UserInfoEncodeSet;
    use Password = self::PasswordEncodeSet;
    use Username = self::UsernameEncodeSet;
    for byte in input.bytes() {
        if byte < 0x20 || byte > 0x7E || match byte as char {
            ' ' | '"' | '#' | '<' | '>' | '?' | '`'
            => is_match!(encode_set, Default | UserInfo | Password | Username),
            '@'
            => is_match!(encode_set, UserInfo | Password | Username),
            '/' | '\\'
            => is_match!(encode_set, Password | Username),
            ':'
            => is_match!(encode_set, Username),
            _ => false,
        } {
            percent_encode_byte(byte, output)
        } else {
            unsafe { str::raw::push_byte(output, byte) }
        }
    }
}


#[inline]
fn percent_encode_byte(byte: u8, output: &mut ~str) {
    unsafe {
        str::raw::push_bytes(output, [
            '%' as u8, to_hex_upper(byte >> 4), to_hex_upper(byte & 0x0F)
        ])
    }
}


#[inline]
fn percent_decode(input: &[u8]) -> ~[u8] {
    let mut output = ~[];
    let mut i = 0u;
    while i < input.len() {
        let c = input[i];
        if c == ('%' as u8) && i + 2 < input.len() {
            match (from_hex(input[i + 1]), from_hex(input[i + 2])) {
                (Some(h), Some(l)) => {
                    output.push(h * 0x10 + l);
                    i += 3;
                    continue
                },
                _ => (),
            }
        }

        output.push(c);
        i += 1;
    }
    output
}
