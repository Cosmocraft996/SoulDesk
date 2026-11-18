//! 本地资源权限的细分模式。旧assets.local保持原行为，不影响其他业务权限。
use crate::store::AppResult;

pub(crate) const SCOPED: &str = "assets.scoped";
pub(crate) const STYLES: &str = "styles.inline";
pub(crate) const IMAGES: &str = "images.blob";
pub(crate) const FETCH: &str = "assets.fetch";
pub(crate) const WASM: &str = "runtime.wasm";
pub(crate) const EVAL: &str = "runtime.eval";
pub(crate) const WORKER: &str = "workers.local";
pub(crate) const BLOB_WORKER: &str = "workers.blob";
const FEATURES: &[&str] = &[STYLES, IMAGES, FETCH, WASM, EVAL, WORKER, BLOB_WORKER];

pub(crate) fn validate(capabilities: &[String]) -> AppResult<()> {
    let has = |name| capabilities.iter().any(|item| item == name);
    if has(SCOPED) && has("assets.local") {
        return Err("CAPABILITY_DENIED: 细分模式不能同时声明旧本地资源大权限".into());
    }
    if !has(SCOPED) && FEATURES.iter().any(|name| has(*name)) {
        return Err("CAPABILITY_DENIED: 细分本地能力须同时声明assets.scoped".into());
    }
    Ok(())
}

pub(crate) fn apply(mut policy: String, capabilities: &[String]) -> String {
    let has = |name| capabilities.iter().any(|item| item == name);
    if validate(capabilities).is_err() {
        return policy;
    }
    if has("assets.local") {
        // 兼容旧包：保留原有展开顺序和取值，不修改已安装版本的授权。
        policy = policy
            .replace(
                "script-src 'self'",
                "script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'",
            )
            .replace("style-src 'self'", "style-src 'self' 'unsafe-inline'")
            .replace("img-src 'self' data:", "img-src 'self' blob: data:")
            .replace("connect-src 'none'", "connect-src 'self'");
        policy.push_str("; worker-src 'self' blob:");
    } else if has(SCOPED) {
        let mut script = "script-src 'self'".to_owned();
        if has(WASM) {
            script.push_str(" 'wasm-unsafe-eval'");
        }
        // 浏览器的unsafe-eval也允许WASM；此隐含能力不能伪称为已关闭。
        if has(EVAL) {
            script.push_str(" 'unsafe-eval'");
        }
        policy = policy.replace("script-src 'self'", &script);
        if has(STYLES) && !policy.contains("style-src 'self' 'unsafe-inline'") {
            policy = policy.replace("style-src 'self'", "style-src 'self' 'unsafe-inline'");
        }
        if has(IMAGES) {
            policy = policy.replace("img-src 'self' data:", "img-src 'self' blob: data:");
        }
        if has(FETCH) {
            policy = policy.replace("connect-src 'none'", "connect-src 'self'");
        }
        let mut workers = Vec::new();
        if has(WORKER) {
            workers.push("'self'");
        }
        if has(BLOB_WORKER) {
            workers.push("blob:");
        }
        // 显式拒绝未声明的线程，避免worker-src缺省时回退到script-src的同源授权。
        policy.push_str(&format!(
            "; worker-src {}",
            if workers.is_empty() {
                "'none'".into()
            } else {
                workers.join(" ")
            }
        ));
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::COMPONENT_CSP;
    fn caps(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn old_package_policy_remains_identical_and_new_mode_starts_closed() {
        let original = COMPONENT_CSP
            .replace(
                "script-src 'self'",
                "script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'",
            )
            .replace("style-src 'self'", "style-src 'self' 'unsafe-inline'")
            .replace("img-src 'self' data:", "img-src 'self' blob: data:")
            .replace("connect-src 'none'", "connect-src 'self'")
            + "; worker-src 'self' blob:";
        assert_eq!(
            apply(COMPONENT_CSP.into(), &caps(&["assets.local"])),
            original
        );
        assert_eq!(apply(COMPONENT_CSP.into(), &[]), COMPONENT_CSP);
        assert_eq!(
            apply(COMPONENT_CSP.into(), &caps(&[SCOPED])),
            format!("{COMPONENT_CSP}; worker-src 'none'")
        );
    }

    #[test]
    fn each_feature_only_changes_its_own_browser_directive() {
        for (feature, directive) in [
            (STYLES, "style-src"),
            (IMAGES, "img-src"),
            (FETCH, "connect-src"),
            (WASM, "script-src"),
            (EVAL, "script-src"),
            (WORKER, "worker-src"),
            (BLOB_WORKER, "worker-src"),
        ] {
            let policy = apply(COMPONENT_CSP.into(), &caps(&[SCOPED, feature]));
            for item in COMPONENT_CSP.split("; ") {
                if !item.starts_with(directive) {
                    assert!(
                        policy.split("; ").any(|actual| actual == item),
                        "{feature}改动了{item}"
                    );
                }
            }
            assert!(!policy.contains("https:"));
            assert_eq!(policy.contains("worker-src 'self'"), feature == WORKER);
            assert_eq!(policy.contains("worker-src blob:"), feature == BLOB_WORKER);
            assert_eq!(policy.contains("'unsafe-eval'"), feature == EVAL);
        }
    }

    #[test]
    fn mixed_or_unmarked_permissions_are_rejected_without_regranting_legacy_access() {
        for names in [
            &[SCOPED, "assets.local"][..],
            &[EVAL][..],
            &[WASM, "assets.local"][..],
        ] {
            assert!(validate(&caps(names)).is_err());
            assert_eq!(apply(COMPONENT_CSP.into(), &caps(names)), COMPONENT_CSP);
        }
    }
}
