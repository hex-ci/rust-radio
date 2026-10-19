//! 云听接口的签名算法。
//!
//! 复刻 App 内 `ParamSignUtils`：按接口路径分组取密钥，参数名 ASCII 升序拼接后加时间戳做 md5。
//! 参数值**不参与 URL 编码**（与查询串编码是两回事）；`timestamp` 必须与请求头一致。

// 官方 Android 客户端使用的接口分组密钥，仅用于兼容访问；不是用户凭据。
const KEY_FORMAT: &str = "40359b279ed420d0643e12df721e6c4d";
const KEY_CONTENT_BIZ: &str = "3196d2bb67a730342382ce8f690c8c44";
const KEY_SEARCH: &str = "68e251be8b49f462be367df22b212ee3";
const KEY_SERIES: &str = "58325b785b752dd7d9c0281dd2befa6a";
const KEY_PUBLISH: &str = "c1e49740f86d3126561c51131c4e384b";
const KEY_LIVE: &str = "732abfe93639060ea6fc8a5f406d08a1";
const KEY_NEWS: &str = "652e16dd26f38ac3d306b31e4839a8e3";

/// 按路径特征选分组密钥：路径含 `/<组>/` 即命中，其余归 `news`。
pub fn group_key(path: &str) -> &'static str {
    const GROUPS: [(&str, &str); 6] = [
        ("format", KEY_FORMAT),
        ("contentBiz", KEY_CONTENT_BIZ),
        ("search", KEY_SEARCH),
        ("series", KEY_SERIES),
        ("publish", KEY_PUBLISH),
        ("live", KEY_LIVE),
    ];
    for (name, key) in GROUPS {
        if path.contains(&format!("/{name}/")) {
            return key;
        }
    }
    KEY_NEWS
}

/// `sign = md5(参数名升序拼接的 k=v&… [末尾已带 &] + timestamp=<毫秒>&key=<分组密钥>)`
pub fn sign(path: &str, params: &[(&str, &str)], timestamp: &str) -> String {
    let mut pairs: Vec<(&str, &str)> = params.to_vec();
    pairs.sort_by(|a, b| a.0.cmp(b.0));

    let mut raw = String::new();
    for (k, v) in &pairs {
        raw.push_str(k);
        raw.push('=');
        raw.push_str(v);
        raw.push('&');
    }
    raw.push_str("timestamp=");
    raw.push_str(timestamp);
    raw.push_str("&key=");
    raw.push_str(group_key(path));
    format!("{:x}", md5::compute(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与接口文档 2.2 的对拍向量一致（固定 ts=1792312000000）
    #[test]
    fn matches_reference_vectors() {
        let ts = "1792312000000";
        assert_eq!(
            sign("/contentBiz/appBroadcast/allByProvince", &[], ts),
            "b5fab53571e471595a07411e3a5cdfa3"
        );
        assert_eq!(
            sign(
                "/contentBiz/appBroadcast/allByProvince",
                &[("provinceCode", "110000")],
                ts
            ),
            "9e85729c120fe025e81ad010d9ec2126"
        );
        assert_eq!(
            sign("/test/unknown", &[("searchWord", "交通")], ts),
            "327120d4865c266ac3842fb52905ff77"
        );
        assert_eq!(
            sign("/format/appProvince/list/all", &[], ts),
            "729b81f07043cbb8ec02060648fdc201"
        );
    }

    #[test]
    fn routes_by_path_containment() {
        assert_eq!(
            group_key("/contentBiz/appBroadcast/detail/639"),
            KEY_CONTENT_BIZ
        );
        assert_eq!(
            group_key("/contentBiz/v1/appProgram/listByDate"),
            KEY_CONTENT_BIZ
        );
        assert_eq!(group_key("/format/appProvince/list/all"), KEY_FORMAT);
        assert_eq!(
            group_key("/search/search/findSearchResourceList"),
            KEY_SEARCH
        );
        assert_eq!(group_key("/whatever"), KEY_NEWS);
    }
}
