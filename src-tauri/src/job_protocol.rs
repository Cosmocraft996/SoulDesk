//! 固定AI直聘数据、网址与评分协议；不创建网页、执行脚本或请求模型。
use crate::store::AppResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) const PACKAGE: &str = "local.souldesk.widgetjs.ai-job";
pub(crate) const CAPABILITY: &str = "jobs.manage";
pub(crate) const CAPTURE_SCRIPT: &str =
    include_str!("../../components/widgetjs/job-source/capture.js");
pub(crate) const LOGIN_SCRIPT: &str = include_str!("../../components/widgetjs/job-source/login.js");
/// 原组件已有的教程及模型申请文档；前端只传固定主题，不传任意网址。
pub(crate) fn help_url(topic: &str) -> AppResult<url::Url> {
    let value = match topic {
        "tutorial" => "https://www.bilibili.com/video/BV1kSPTe4EZw/",
        "deepseek" => "https://platform.deepseek.com/usage",
        "qwen" => "https://help.aliyun.com/zh/model-studio/getting-started/first-api-call-to-qwen",
        "spark" => "https://xinghuo.xfyun.cn/sparkapi",
        "tencent" => "https://cloud.tencent.com/document/product/1772/115970",
        _ => return Err("INVALID_ARGUMENT: 未知AI直聘帮助主题".into()),
    };
    url::Url::parse(value).map_err(|_| "DATA_UNAVAILABLE: 固定帮助网址无效".into())
}
pub(crate) fn login_state(value: Value) -> AppResult<bool> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Login {
        origin: String,
        logged_in: bool,
    }
    let login: Login =
        serde_json::from_value(value).map_err(|_| "DATA_UNAVAILABLE: 登录状态尚未就绪")?;
    if login.origin != "https://www.zhipin.com" {
        return Err("PRESENTATION_EXPIRED: 登录状态不属于当前BOSS页面".into());
    }
    Ok(login.logged_in)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Search {
    pub skills: String,
    pub self_introduction: String,
    pub desired_salary: String,
    pub location: String,
    pub search_keyword: String,
    pub disadvantages: String,
    pub advantages: Vec<String>,
}
impl Default for Search {
    fn default() -> Self {
        Self {
            skills: String::new(),
            self_introduction: String::new(),
            desired_salary: String::new(),
            location: String::new(),
            search_keyword: String::new(),
            disadvantages: String::new(),
            advantages: vec!["双休".into(), "不加班".into(), "早九晚五".into()],
        }
    }
}
fn text(value: &str, max: usize) -> bool {
    value.chars().count() <= max
        && !value
            .chars()
            .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
}
impl Search {
    pub(crate) fn validate(&self, starting: bool) -> AppResult<()> {
        let mut tags = HashSet::new();
        if !text(&self.search_keyword, 120)
            || starting && self.search_keyword.trim().is_empty()
            || !text(&self.skills, 2000)
            || !text(&self.self_introduction, 2000)
            || !text(&self.desired_salary, 200)
            || !text(&self.location, 300)
            || !text(&self.disadvantages, 2000)
            || self.advantages.len() > 32
            || self
                .advantages
                .iter()
                .any(|v| v.trim().is_empty() || !text(v, 100) || !tags.insert(v))
        {
            return Err("INVALID_ARGUMENT: 求职条件为空、超长或包含无效字段".into());
        }
        Ok(())
    }
}
pub(crate) fn job_id(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        Err("INVALID_ARGUMENT: 职位标识无效".into())
    } else {
        Ok(())
    }
}
pub(crate) fn site_navigation(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|h| h == "zhipin.com" || h.ends_with(".zhipin.com"))
}
pub(crate) fn search_url(keyword: &str, page: u32) -> AppResult<url::Url> {
    if keyword.trim().is_empty() || !text(keyword, 120) || !(1..=10000).contains(&page) {
        return Err("INVALID_ARGUMENT: 搜索关键字或页码无效".into());
    }
    let mut url = url::Url::parse("https://www.zhipin.com/web/geek/job").expect("固定网址有效");
    url.query_pairs_mut()
        .append_pair("query", keyword)
        .append_pair("page", &page.to_string());
    Ok(url)
}
pub(crate) fn detail_url(id: &str) -> AppResult<url::Url> {
    job_id(id)?;
    url::Url::parse(&format!("https://www.zhipin.com/job_detail/{id}.html"))
        .map_err(|_| "INVALID_ARGUMENT: 职位网址无效".into())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Job {
    pub encrypt_job_id: String,
    pub area_district: String,
    pub brand_name: String,
    pub brand_logo: String,
    pub job_name: String,
    pub job_labels: Vec<String>,
    pub skills: Vec<String>,
    pub job_experience: String,
    pub salary_desc: String,
    pub job_degree: String,
    pub city_name: String,
    pub business_district: String,
    pub brand_stage_name: String,
    pub brand_industry: String,
    pub brand_scale_name: String,
    pub welfare_list: Vec<String>,
    pub days_per_week_desc: String,
    pub least_month_desc: String,
}
impl Job {
    /// 仅内部固定采集结果使用；额外站点字段不会保存或发给模型。
    pub(crate) fn from_page(value: Value) -> AppResult<Self> {
        let fields = value
            .as_object()
            .ok_or("DATA_UNAVAILABLE: 职位必须为对象")?;
        let string = |key: &str, max: usize| -> AppResult<String> {
            match fields.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) if text(s, max) => Ok(s.clone()),
                _ => Err(format!("DATA_UNAVAILABLE: 职位{key}字段无效")),
            }
        };
        let list = |key: &str| -> AppResult<Vec<String>> {
            match fields.get(key) {
                None | Some(Value::Null) => Ok(Vec::new()),
                Some(Value::Array(a)) if a.len() <= 100 => a
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .filter(|s| text(s, 300))
                            .map(str::to_owned)
                            .ok_or_else(|| format!("DATA_UNAVAILABLE: 职位{key}字段无效"))
                    })
                    .collect(),
                _ => Err(format!("DATA_UNAVAILABLE: 职位{key}字段无效")),
            }
        };
        let job = Self {
            encrypt_job_id: string("encryptJobId", 160)?,
            area_district: string("areaDistrict", 300)?,
            brand_name: string("brandName", 300)?,
            brand_logo: string("brandLogo", 2048)?,
            job_name: string("jobName", 500)?,
            job_labels: list("jobLabels")?,
            skills: list("skills")?,
            job_experience: string("jobExperience", 300)?,
            salary_desc: string("salaryDesc", 300)?,
            job_degree: string("jobDegree", 300)?,
            city_name: string("cityName", 300)?,
            business_district: string("businessDistrict", 300)?,
            brand_stage_name: string("brandStageName", 300)?,
            brand_industry: string("brandIndustry", 300)?,
            brand_scale_name: string("brandScaleName", 300)?,
            welfare_list: list("welfareList")?,
            days_per_week_desc: string("daysPerWeekDesc", 300)?,
            least_month_desc: string("leastMonthDesc", 300)?,
        };
        job_id(&job.encrypt_job_id).map_err(|_| "DATA_UNAVAILABLE: 职位标识无效")?;
        if job.job_name.trim().is_empty() {
            return Err("DATA_UNAVAILABLE: 职位名缺失".into());
        }
        Ok(job)
    }
    pub(crate) fn ai_value(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("固定职位可序列化");
        value.as_object_mut().expect("职位对象").remove("brandLogo");
        value
    }
}
#[derive(Debug)]
pub(crate) struct Page {
    pub jobs: Vec<Job>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_next: bool,
}
pub(crate) fn parse_page(value: Value, expected_page: u32) -> AppResult<Page> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Raw {
        origin: String,
        path: String,
        logged_in: bool,
        page: u32,
        page_size: u32,
        total: u64,
        jobs: Vec<Value>,
    }
    match value["error"].as_str() {
        Some("login_required") => return Err("LOGIN_REQUIRED: 请先手动登录BOSS账号".into()),
        Some(_) => return Err("DATA_UNAVAILABLE: 页面尚未提供固定职位结构".into()),
        None => (),
    }
    if serde_json::to_vec(&value)
        .map_err(|_| "DATA_UNAVAILABLE: 页面格式无效")?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("CAPACITY_EXCEEDED: 单页职位超过容量".into());
    }
    let raw: Raw = serde_json::from_value(value).map_err(|_| "DATA_UNAVAILABLE: 页面元数据无效")?;
    if raw.origin != "https://www.zhipin.com"
        || raw.path != "/web/geek/job"
        || !raw.logged_in
        || raw.page != expected_page
        || !(1..=10000).contains(&raw.page)
        || !(1..=100).contains(&raw.page_size)
        || raw.jobs.len() > raw.page_size as usize
        || raw.total > 9_007_199_254_740_991
    {
        return Err("PRESENTATION_EXPIRED: 职位页面、登录或页码与当前任务不符".into());
    }
    if raw.jobs.is_empty() && raw.total > 0 {
        return Err("DATA_UNAVAILABLE: 页面总量非零但职位尚未载入".into());
    }
    let offset = (u64::from(raw.page) - 1) * u64::from(raw.page_size);
    if raw.jobs.len() as u64
        != raw
            .total
            .saturating_sub(offset)
            .min(u64::from(raw.page_size))
    {
        return Err("DATA_UNAVAILABLE: 职位列表尚未完整载入，未跳过本页".into());
    }
    if !raw.jobs.is_empty()
        && (u64::from(raw.page) - 1) * u64::from(raw.page_size) + raw.jobs.len() as u64 > raw.total
    {
        return Err("DATA_UNAVAILABLE: 页面职位数量与总量不符".into());
    }
    let jobs = raw
        .jobs
        .into_iter()
        .map(Job::from_page)
        .collect::<AppResult<Vec<_>>>()?;
    let mut ids = HashSet::new();
    if jobs.iter().any(|j| !ids.insert(&j.encrypt_job_id)) {
        return Err("DATA_UNAVAILABLE: 单页包含重复职位".into());
    }
    Ok(Page {
        jobs,
        page: raw.page,
        page_size: raw.page_size,
        total: raw.total,
        has_next: u64::from(raw.page) * u64::from(raw.page_size) < raw.total,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Rating {
    pub salary_match: bool,
    pub degree_match: bool,
    pub encrypt_job_id: String,
    pub comment: String,
    pub score: u8,
}
pub(crate) fn ratings(text_value: &str, expected: &[String]) -> AppResult<Vec<Rating>> {
    if text_value.len() > 512 * 1024 || expected.is_empty() || expected.len() > 100 {
        return Err("DATA_UNAVAILABLE: 职位评分数量或容量无效".into());
    }
    let mut wanted = HashSet::new();
    for id in expected {
        job_id(id)?;
        if !wanted.insert(id.as_str()) {
            return Err("INVALID_ARGUMENT: 请求职位重复".into());
        }
    }
    // 直接解析原JSON，避免先转Value时重复字段被覆盖。两种格式都整批严格校验。
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wrapped {
        jobs: Vec<Rating>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Output {
        Array(Vec<Rating>),
        Wrapped(Wrapped),
    }
    let results = match serde_json::from_str::<Output>(text_value)
        .map_err(|_| "DATA_UNAVAILABLE: 模型评分JSON结构、字段或类型无效")?
    {
        Output::Array(items) => items,
        Output::Wrapped(wrapper) => wrapper.jobs,
    };
    let mut found = HashSet::new();
    if results.len() != wanted.len()
        || results.iter().any(|r| {
            !wanted.contains(r.encrypt_job_id.as_str())
                || !found.insert(r.encrypt_job_id.as_str())
                || !(1..=5).contains(&r.score)
                || r.comment.trim().is_empty()
                || !text(&r.comment, 2000)
        })
    {
        return Err("DATA_UNAVAILABLE: 模型评分遗漏、重复、越界或混入其他职位，未写入".into());
    }
    Ok(results)
}
pub(crate) fn rating_body(model: &str, search: &Search, jobs: &[Job]) -> AppResult<Value> {
    search.validate(true)?;
    if model.trim().is_empty()
        || !text(model, 200)
        || model.chars().any(char::is_control)
        || jobs.is_empty()
        || jobs.len() > 100
    {
        return Err("INVALID_ARGUMENT: 模型或待分析职位无效".into());
    }
    let mut ids = HashSet::new();
    if jobs
        .iter()
        .any(|j| job_id(&j.encrypt_job_id).is_err() || !ids.insert(&j.encrypt_job_id))
    {
        return Err("INVALID_ARGUMENT: 待分析职位标识无效或重复".into());
    }
    let prompts: Value = serde_json::from_str(include_str!(
        "../../components/widgetjs/job-source/prompts.json"
    ))
    .map_err(|_| "AI直聘原提示词资源无效")?;
    let mut profile = vec!["我的个人信息如下：".to_owned()];
    if !search.self_introduction.is_empty() {
        profile.push(format!("我的简介标签：{}", search.self_introduction));
    }
    if !search.skills.is_empty() {
        profile.push(format!("技能标签：{}", search.skills));
    }
    profile.push("我的求职意向：".into());
    profile.push(format!("{{我希望找到的职位：{}。", search.search_keyword));
    if !search.desired_salary.is_empty() {
        profile.push(format!("期望薪资范围：{}。", search.desired_salary));
    }
    if !search.advantages.is_empty() {
        profile.push(format!(
            "desiredJobAdvantages：{}。",
            search.advantages.join(" ")
        ));
    }
    if !search.location.is_empty() {
        profile.push(format!("期望工作地点：{}。", search.location));
    }
    if !search.disadvantages.is_empty() {
        profile.push(format!("jobDisadvantages：{}。", search.disadvantages));
    }
    profile.push("}".into());
    let body = json!({"model":model.trim(),"stream":true,"response_format":{"type":"json_object"},"max_tokens":8192,
        "messages":[{"role":"assistant","content":prompts["jobAnalysis"]},{"role":"user","content":profile.join("\n")},{"role":"user","content":serde_json::to_string(&jobs.iter().map(Job::ai_value).collect::<Vec<_>>()).map_err(|_|"职位输入编码失败")?},
        {"role":"user","content":"为兼容JSON对象输出模式，请将上述评分数组放入唯一的jobs字段，返回{\"jobs\":[全部职位评分]}。每项保留原字段与原始encryptJobId，不添加说明、代码围栏或其他字段。"}]});
    if serde_json::to_vec(&body)
        .map_err(|_| "评分请求编码失败")?
        .len()
        > 64 * 1024
    {
        return Err("CAPACITY_EXCEEDED: 评分输入超过64KiB，请按完整职位分批，未截断".into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn search() -> Search {
        Search {
            search_keyword: "前端 & C++ #远程".into(),
            ..Search::default()
        }
    }
    fn job(id: &str) -> Job {
        Job::from_page(json!({"encryptJobId":id,"jobName":"前端工程师","skills":["Vue","TypeScript"],"brandLogo":"https://img.example/logo.png","salaryDesc":"15-25K","uid":"不可留存","cookie":"不可留存"})).unwrap()
    }
    fn page() -> Value {
        json!({"origin":"https://www.zhipin.com","path":"/web/geek/job","loggedIn":true,"page":1,"pageSize":1,"total":2,"jobs":[serde_json::to_value(job("a-b_1")).unwrap()]})
    }
    fn rating(id: &str) -> Value {
        json!({"salaryMatch":true,"degreeMatch":false,"encryptJobId":id,"comment":"技能相符，需核实经验要求","score":4})
    }

    #[test]
    fn search_preserves_original_defaults_and_rejects_unrequested_fields() {
        let s = Search::default();
        assert_eq!(s.advantages, ["双休", "不加班", "早九晚五"]);
        s.validate(false).unwrap();
        assert!(s.validate(true).is_err());
        for key in ["script", "apiKey", "url", "instanceId"] {
            let mut v = serde_json::to_value(search()).unwrap();
            v[key] = json!("不接受");
            assert!(serde_json::from_value::<Search>(v).is_err());
        }
        let mut s = search();
        s.advantages.push("双休".into());
        assert!(s.validate(true).is_err());
        s = search();
        s.self_introduction = "行\n文\t有效".into();
        s.validate(true).unwrap();
        s.skills = "a\0b".into();
        assert!(s.validate(false).is_err());
        s = search();
        s.search_keyword = "字".repeat(121);
        assert!(s.validate(false).is_err());
    }
    #[test]
    fn fixed_urls_encode_keywords_and_reject_path_or_origin_injection() {
        let s = search();
        let u = search_url(&s.search_keyword, 2).unwrap();
        assert_eq!(u.origin().ascii_serialization(), "https://www.zhipin.com");
        assert_eq!(u.path(), "/web/geek/job");
        assert_eq!(
            u.query_pairs().collect::<Vec<_>>(),
            [
                ("query".into(), s.search_keyword.into()),
                ("page".into(), "2".into())
            ]
        );
        assert!(u.fragment().is_none());
        for id in [
            "../x",
            "x/y",
            "x?z=1",
            "x#y",
            "",
            "中文",
            "https://evil.example",
        ] {
            assert!(detail_url(id).is_err());
        }
        assert_eq!(
            detail_url("a-b_1").unwrap().as_str(),
            "https://www.zhipin.com/job_detail/a-b_1.html"
        );
        for page in [0, 10001, u32::MAX] {
            assert!(search_url("开发", page).is_err());
        }
        for u in [
            "https://www.zhipin.com/",
            "https://login.zhipin.com/a",
            "https://zhipin.com/",
        ] {
            assert!(site_navigation(&url::Url::parse(u).unwrap()));
        }
        for u in [
            "http://www.zhipin.com/",
            "https://www.zhipin.com:444/",
            "https://zhipin.com.evil.example/",
            "https://evilzhipin.com/",
            "https://user@www.zhipin.com/",
            "file:///C:/x",
            "javascript:alert(1)",
        ] {
            assert!(!site_navigation(&url::Url::parse(u).unwrap()));
        }
    }
    #[test]
    fn captured_fields_are_explicit_and_bad_types_are_not_coerced() {
        let j = job("id1");
        assert_eq!(j.skills, ["Vue", "TypeScript"]);
        assert!(j.welfare_list.is_empty());
        let saved = serde_json::to_value(&j).unwrap();
        assert_eq!(saved.as_object().unwrap().len(), 18);
        assert!(saved.get("uid").is_none());
        assert!(saved.get("cookie").is_none());
        assert!(j.ai_value().get("brandLogo").is_none());
        for bad in [
            json!({"encryptJobId":"id1"}),
            json!({"encryptJobId":"id1","jobName":"开发","skills":[1]}),
            json!({"encryptJobId":"id1","jobName":"开发","salaryDesc":100}),
            json!({"encryptJobId":"../x","jobName":"开发"}),
        ] {
            assert!(Job::from_page(bad).is_err());
        }
    }
    #[test]
    fn page_requires_matching_identity_and_complete_metadata() {
        let p = parse_page(page(), 1).unwrap();
        assert_eq!(
            (p.page, p.page_size, p.total, p.has_next, p.jobs.len()),
            (1, 1, 2, true, 1)
        );
        for (field, bad) in [
            ("origin", json!("https://login.zhipin.com")),
            ("path", json!("/job_detail/x.html")),
            ("loggedIn", json!(false)),
            ("page", json!(2)),
            ("pageSize", json!(0)),
            ("total", json!(9_007_199_254_740_992_u64)),
            ("total", json!("31")),
        ] {
            let mut v = page();
            v[field] = bad;
            assert!(parse_page(v, 1).is_err(), "{field}");
        }
        let mut v = page();
        v["script"] = json!("不接受");
        assert!(parse_page(v, 1).is_err());
        assert!(parse_page(json!({"error":"login_required"}), 1)
            .unwrap_err()
            .starts_with("LOGIN_REQUIRED:"));
        assert!(parse_page(json!({"error":"not_ready"}), 1)
            .unwrap_err()
            .starts_with("DATA_UNAVAILABLE:"));
    }
    #[test]
    fn empty_or_repeated_capture_never_reports_a_successful_full_page() {
        let mut v = page();
        v["jobs"] = json!([]);
        assert!(parse_page(v.clone(), 1).is_err());
        v["total"] = json!(0);
        let p = parse_page(v, 1).unwrap();
        assert!(p.jobs.is_empty() && !p.has_next);
        let mut v = page();
        v["pageSize"] = json!(2);
        v["jobs"] = json!([job("id1"), job("id1")]);
        assert!(parse_page(v, 1).is_err());
        let mut v = page();
        v["pageSize"] = json!(1);
        v["jobs"] = json!([job("id1"), job("id2")]);
        assert!(parse_page(v, 1).is_err());
    }
    #[test]
    fn scores_accept_original_array_or_single_json_mode_wrapper() {
        let items = json!([rating("id2"), rating("id1")]);
        let wanted = vec!["id1".into(), "id2".into()];
        let a = ratings(&items.to_string(), &wanted).unwrap();
        assert_eq!(a[0].encrypt_job_id, "id2");
        assert_eq!(a[0].score, 4);
        assert_eq!(
            ratings(&json!({"jobs":items}).to_string(), &wanted).unwrap(),
            a
        );
    }
    #[test]
    fn partial_foreign_repeated_or_malformed_score_batches_are_rejected_whole() {
        let wanted = vec!["id1".into(), "id2".into()];
        for bad in [
            json!([rating("id1")]),
            json!([rating("id1"), rating("foreign")]),
            json!([rating("id1"), rating("id1")]),
            json!({"jobs":[rating("id1"),rating("id2")],"other":true}),
            json!({"results":[rating("id1"),rating("id2")]}),
        ] {
            assert!(ratings(&bad.to_string(), &wanted).is_err());
        }
        for (key, bad) in [
            ("score", json!(0)),
            ("score", json!(6)),
            ("score", json!(3.5)),
            ("score", json!("4")),
            ("salaryMatch", json!(1)),
            ("degreeMatch", json!(null)),
            ("comment", json!("  ")),
            ("comment", json!("字".repeat(2001))),
            ("script", json!("不接受")),
        ] {
            let mut r = rating("id1");
            r[key] = bad;
            assert!(
                ratings(&json!([r, rating("id2")]).to_string(), &wanted).is_err(),
                "{key}"
            );
        }
        assert!(ratings("```json\n[]\n```", &wanted).is_err());
        assert!(ratings(&" ".repeat(512 * 1024 + 1), &wanted).is_err());
        assert!(ratings("[]", &[]).is_err());
        assert!(ratings("[]", &["id1".into(), "id1".into()]).is_err());
    }
    #[test]
    fn rating_request_preserves_original_prompt_roles_and_complete_job_identity() {
        let mut s = search();
        s.skills = "Vue\nRust".into();
        s.self_introduction = "五年经验".into();
        s.location = "杭州".into();
        let body = rating_body("my-model", &s, &[job("id1")]).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|m| m["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["assistant", "user", "user", "user"]
        );
        let prompts: Value = serde_json::from_str(include_str!(
            "../../components/widgetjs/job-source/prompts.json"
        ))
        .unwrap();
        assert_eq!(messages[0]["content"], prompts["jobAnalysis"]);
        assert!(messages[1]["content"].as_str().unwrap().contains("杭州"));
        let jobs: Value = serde_json::from_str(messages[2]["content"].as_str().unwrap()).unwrap();
        assert_eq!(jobs[0]["encryptJobId"], "id1");
        assert!(jobs[0].get("brandLogo").is_none());
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 8192);
        for model in ["", "x\ny"] {
            assert!(rating_body(model, &s, &[job("id1")]).is_err());
        }
        assert!(rating_body("my-model", &s, &[job("id1"), job("id1")]).is_err());
    }
    #[test]
    fn large_model_input_is_rejected_without_silent_truncation() {
        let jobs = (0..100)
            .map(|i| {
                let mut value = job(&format!("id{i}"));
                value.skills = vec!["字".repeat(300); 10];
                value
            })
            .collect::<Vec<_>>();
        assert!(
            matches!(rating_body("m", &search(), &jobs),Err(e) if e.starts_with("CAPACITY_EXCEEDED:"))
        );
        assert!(rating_body("m", &search(), &[]).is_err());
    }
    #[test]
    fn duplicate_json_fields_and_inconsistent_page_totals_are_rejected() {
        let repeated = r#"[{"salaryMatch":true,"degreeMatch":true,"encryptJobId":"id1","comment":"完整评分","score":1,"score":5}]"#;
        assert!(ratings(repeated, &["id1".into()]).is_err());
        let repeated = format!(
            r#"{{"jobs":[{}],"jobs":[{}]}}"#,
            rating("id1"),
            rating("id1")
        );
        assert!(ratings(&repeated, &["id1".into()]).is_err());
        let mut v = page();
        v["total"] = json!(0);
        assert!(parse_page(v, 1).is_err());
    }
    #[test]
    fn location_is_sent_even_without_benefit_tags_and_json_mode_has_a_matching_instruction() {
        let mut s = search();
        s.advantages.clear();
        s.location = "杭州".into();
        let b = rating_body("m", &s, &[job("id1")]).unwrap();
        assert!(b["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("期望工作地点：杭州"));
        assert!(b["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("唯一的jobs字段"));
        assert_eq!(b["response_format"]["type"], "json_object");
    }
    #[test]
    fn login_reply_accepts_only_the_fixed_origin_and_boolean_without_account_fields() {
        assert!(login_state(json!({"origin":"https://www.zhipin.com","loggedIn":true})).unwrap());
        assert!(!login_state(json!({"origin":"https://www.zhipin.com","loggedIn":false})).unwrap());
        for bad in [
            json!({"origin":"https://login.zhipin.com","loggedIn":true}),
            json!({"origin":"https://www.zhipin.com","loggedIn":"true"}),
            json!({"origin":"https://www.zhipin.com","loggedIn":true,"uid":"不接受"}),
            json!({"error":"not_ready"}),
        ] {
            assert!(login_state(bad).is_err());
        }
    }
    #[test]
    fn a_partial_page_cannot_be_mistaken_for_a_complete_page_and_advance_the_cursor() {
        let mut v = page();
        v["pageSize"] = json!(30);
        v["total"] = json!(31);
        assert!(parse_page(v, 1).unwrap_err().contains("未跳过本页"));
    }
}
