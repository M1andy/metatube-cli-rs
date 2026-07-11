/// Replicates `number.Trim()` and `number.IsUncensored()` from
/// `common/number/number.go` in the Go metatube-sdk-go project.
use regex::Regex;
use std::sync::LazyLock;

static EXT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.[a-z\d]{1,7}$").unwrap());
static DOMAIN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([a-z\d]+\.(?:com|net|top|xyz|tv))(?:[^a-z\d]|$)").unwrap());
static NUM_DASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([a-z\d]+(?:[-_][a-z\d]{2,})+)").unwrap());
static NUM_ALPHA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)((?:[a-z]+\d|\d+[a-z])[a-z\d]+)").unwrap());
static PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:f?hd|sd)[-_](.*$)").unwrap());
static TAGS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[-_.](dvd|iso|mkv|mp4|c?avi|\d*fps|whole|(f|hhb)?hd\d*|sd\d*|(?:360|480|720|1080|2160)[pi]|X1080X|uncensored|leak|[2468]ks?|[xh]26[45])+").unwrap()
});
static MAKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|[-_\s]+)(carib(b?ean)?(com)?(pr)?|1?Pond?o?|10mu(sume)?|paco(paco)?(mama)?|mura(mura)?|Tokyo[-_\s]?Hot)([-_\s]+(?P<pattern>\d{4,}[-_]\d{2,}|[a-z]{1,4}\d{2,4})|$)").unwrap()
});
static FC2_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i)\s*(FC2[-_]?PPV)[-_]").unwrap());
static SUFFIX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([-_](c|uc|ch|cd\d{1,2})|hhb\d*|ch|A|B|C|D)\s*$").unwrap()
});
static UNCENSORED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(\d{4,6}[-_]\d{2,3}|(cz|gedo|k|n|kb|se)\d{2,4}|(heyzo|xxx-av|heydouga|kin8)[-_].+)|([hc]0930|h4610|av9898|1000giri)[-_][a-z\d]+$").unwrap()
});

/// Extracts a JAV movie ID from a raw filename string.
/// Ported from `number.Trim()` in common/number/number.go.
pub fn trim(s: &str) -> String {
    let mut s = s.to_string();

    // trim extension (max 7 chars)
    if let Some(caps) = EXT_RE.captures(&s) {
        if let Some(m) = caps.get(0) {
            let ext_len = m.as_str().len();
            if ext_len <= 7 {
                s = s[..s.len() - ext_len].to_string();
            }
        }
    }

    s = DOMAIN_RE.replace_all(&s, "").to_string();

    if let Some(caps) = NUM_DASH_RE.captures(&s) {
        if let Some(m) = caps.get(1) {
            s = m.as_str().to_string();
        }
    } else if let Some(caps) = NUM_ALPHA_RE.captures(&s) {
        if let Some(m) = caps.get(1) {
            s = m.as_str().to_string();
        }
    }

    s = PREFIX_RE.replace_all(&s, "${1}").to_string();
    s = TAGS_RE.replace_all(&s, "").to_string();
    s = MAKER_RE.replace_all(&s, "${pattern}").to_string();
    s = FC2_PREFIX_RE.replace_all(&s, "FC2-").to_string();

    while SUFFIX_RE.is_match(&s) {
        s = SUFFIX_RE.replace_all(&s, "").to_string();
    }

    s.trim().to_string()
}

/// Returns true if the number pattern belongs to an uncensored movie studio.
/// Ported from `number.IsUncensored()` in common/number/number.go.
pub fn is_uncensored(s: &str) -> bool {
    UNCENSORED_RE.is_match(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim() {
        let cases = vec![
            ("", ""),
            ("n9110", "n9110"),
            ("ABP-030", "ABP-030"),
            ("ABP-030-C", "ABP-030"),
            ("ABP-030-UC.mp4", "ABP-030"),
            ("ABP-358_C.mkv", "ABP-358"),
            ("[22sht.me]ABP-358_C.mkv", "ABP-358"),
            ("[98t.tv]vema-181-4k-C.mp4", "vema-181"),
            ("ABP-030-C-c_c-C-Cd1-cd4.mp4", "ABP-030"),
            ("rctd-460ch.mp4", "rctd-460"),
            ("rctd-460-ch.mp4", "rctd-460"),
            ("rctd-460-uc.mp4", "rctd-460"),
            ("FC2-PPV-123456", "FC2-123456"),
            ("FC2PPV-123456", "FC2-123456"),
            ("FC2-PPV-123456-C.mp4", "FC2-123456"),
            ("HD_GS-333", "GS-333"),
            ("FHD-MXGS-247-C", "MXGS-247"),
            ("HDD-697-C-dvd.mp4", "HDD-697"),
            ("MXGS-697.HD.mp4", "MXGS-697"),
            ("MXGS-697-AVI", "MXGS-697"),
            ("10MUSI-234-C.mp4", "10MUSI-234"),
            ("Pond-112-C.mp4", "Pond-112"),
            ("Pono-n8877-C.mp4", "n8877"),
            ("avop-208-hhbhd.mp4", "avop-208"),
            ("SDDE-625_uncensored_C", "SDDE-625"),
            ("SDDE-625_uncensored_leak_C_cd1.mp4", "SDDE-625"),
            ("GIGL-677_4K.mp4", "GIGL-677"),
            ("SSIS-329-C_1080P30FPSFHDx264", "SSIS-329"),
            ("hhd800.com@HUNTB-269", "HUNTB-269"),
            ("jav20s8.com@GIGL-677_4K.mp4", "GIGL-677"),
            ("133ARA-030你好.mp4", "133ARA-030"),
            ("Tokyo Hot n9001 FHD.mp4", "n9001"),
            ("TokyoHot-n1287-HD .mp4", "n1287"),
            ("caribbeancom-020317_001.mp4", "020317_001"),
            ("020317-001-1pondo.mp4", "020317-001"),
            ("020317-001-paco.mp4", "020317-001"),
            ("heydouga-4102-023.mp4", "heydouga-4102-023"),
            ("xxx-av-1789-C.mp4", "xxx-av-1789"),
            ("[HND-620] 絶対にナマで連射させてくれる連続中出しソープ_あいだ飛鳥.mp4", "HND-620"),
            ("MIDV-111-C_X1080X.mp4", "MIDV-111"),
            ("hhd800.com@midv00574hhb_60fps", "midv00574"),
            ("hhd800.com@jums00150hhb", "jums00150"),
            ("GIGL-677_2K_h265.mp4", "GIGL-677"),
            ("093021_539-FHD.mkv", "093021_539"),
            ("093021_539-1080pFHD.mkv", "093021_539"),
        ];

        for (input, expected) in &cases {
            assert_eq!(trim(input), *expected, "trim({:?})", input);
        }
    }

    #[test]
    fn test_is_uncensored() {
        for case in ["123456_789", "heyzo-1342", "n1342", "h4610-tk1003", "xxx-av-1789"] {
            assert!(is_uncensored(case), "{} should be uncensored", case);
        }
        for case in ["ABP-030", "ssis00123", "133ARA-030"] {
            assert!(!is_uncensored(case), "{} should NOT be uncensored", case);
        }
    }
}
