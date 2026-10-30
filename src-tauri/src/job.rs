/// 九职业静态表：基础名、英文家族名、合法一次转职 grow 号与转职中文名。纯表，不读盘。
///
/// 职业顺序出处：`local-run/recon/role-create-jobs-report.md` 第 2 节的 slot==job 表，
/// 同 `Server-70s3a3/src/Tools/VmTestKit/Role-Create-Jobs-FG202.cs:19-27` 的 InitialStatsFG202 行注释顺序。
/// 合法 grow 号与转职名出处：`Server-70s3a3/data/quest-catalog.xml` 里 `JobChange=true` 且
/// `RewardGrowType>0` 的"转职 - X (n/m)"结算步，括号里的任务号就是那一步的 Id。
/// thief 只有 1..2、at mage 只有 1..2：其余 grow 号在任务表里没有对应转职链
/// （`.chr` 里那些 growtype 行是 CN70 内容未采用的分支）。
struct JobRow {
    /// quest-catalog.xml 的 `<Job>` 值（去掉方括号后逐字相等，用于匹配任务表）。
    family: &'static str,
    base: &'static str,
    /// 下标即 grow 号，0 位不用；空串表示该职业没有这个分支。
    advances: [&'static str; 5],
}

const JOBS: [JobRow; 9] = [
    // 0 swordman 鬼剑士：剑魂 806 / 鬼泣 803 / 狂战士 809 / 阿修罗 834
    JobRow { family: "swordman", base: "鬼剑士", advances: ["", "剑魂", "鬼泣", "狂战士", "阿修罗"] },
    // 1 fighter 格斗家：气功师 7817 / 散打 7820 / 街霸 7824 / 柔道家 7827
    JobRow { family: "fighter", base: "格斗家", advances: ["", "气功师", "散打", "街霸", "柔道家"] },
    // 2 gunner 神枪手：漫游枪手 7831 / 枪炮师 7834 / 机械师 7837 / 弹药专家 7840
    JobRow { family: "gunner", base: "神枪手", advances: ["", "漫游枪手", "枪炮师", "机械师", "弹药专家"] },
    // 3 mage 魔法师：元素师 7842 / 召唤师 7845 / 战斗法师 7848 / 魔道学者 7851
    JobRow { family: "mage", base: "魔法师", advances: ["", "元素师", "召唤师", "战斗法师", "魔道学者"] },
    // 4 priest 圣职者：圣骑士 7855 / 蓝拳圣使 7859 / 驱魔师 7862 / 复仇者 7866
    JobRow { family: "priest", base: "圣职者", advances: ["", "圣骑士", "蓝拳圣使", "驱魔师", "复仇者"] },
    // 5 at gunner 女枪手：与神枪手同四名，7870 / 7873 / 7876 / 7879
    JobRow { family: "at gunner", base: "女枪手", advances: ["", "漫游枪手", "枪炮师", "机械师", "弹药专家"] },
    // 6 thief 暗夜使者：刺客 7882 / 死灵术士 7886
    JobRow { family: "thief", base: "暗夜使者", advances: ["", "刺客", "死灵术士", "", ""] },
    // 7 at fighter 男格斗家：与格斗家同四名（7889 / 7892 / 7895 / 7898），靠一次觉醒名区分性别分支
    JobRow { family: "at fighter", base: "男格斗家", advances: ["", "气功师", "散打", "街霸", "柔道家"] },
    // 8 at mage 男魔法师：只有元素爆破师 4065 / 冰结师 4068（魔法师 3 才是元素师/召唤师/…）
    JobRow { family: "at mage", base: "男魔法师", advances: ["", "元素爆破师", "冰结师", "", ""] },
];

fn row(job: i64) -> Option<&'static JobRow> {
    JOBS.get(job as usize)
}

/// 这套客户端可建的职业数量（job 枚举 0..count）。
pub fn count() -> i64 {
    JOBS.len() as i64
}

/// 未知 job 不猜名字，只给一个明确的占位说法（9/10 这套客户端没有 `.chr`）。
pub fn base_name(job: i64) -> String {
    match row(job) {
        Some(value) => value.base.to_string(),
        None => format!("职业 {job}"),
    }
}

pub fn family(job: i64) -> &'static str {
    row(job).map(|value| value.family).unwrap_or_default()
}

pub fn index_of_family(name: &str) -> Option<i64> {
    JOBS.iter()
        .position(|value| value.family == name)
        .map(|value| value as i64)
}

/// 非法 (job, grow) 组合返回空串，调用方据此留白或拒写，而不是编一个不存在的分支名。
pub fn advance_name(job: i64, first: i64) -> &'static str {
    let Some(value) = row(job) else {
        return "";
    };
    if !(1..=4).contains(&first) {
        return "";
    }
    value.advances[first as usize]
}

pub fn allowed(job: i64) -> Vec<i64> {
    (1..=4)
        .filter(|first| !advance_name(job, *first).is_empty())
        .collect()
}

pub fn is_allowed(job: i64, first: i64) -> bool {
    !advance_name(job, first).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_grow_branches_match_the_quest_table() {
        let lengths: Vec<usize> = (0..9).map(|job| allowed(job).len()).collect();
        assert_eq!(lengths, vec![4, 4, 4, 4, 4, 4, 2, 4, 2]);
        assert_eq!(allowed(6), vec![1, 2]);
        assert_eq!(allowed(8), vec![1, 2]);
    }

    #[test]
    fn advance_names_are_scoped_to_their_own_family() {
        assert_eq!(advance_name(0, 1), "剑魂");
        assert_eq!(advance_name(8, 1), "元素爆破师");
        assert_eq!(advance_name(6, 3), "");
        assert!(!is_allowed(8, 4));
        assert!(is_allowed(0, 4));
        assert_eq!(advance_name(0, 0), "");
        assert_eq!(advance_name(0, 5), "");
    }

    #[test]
    fn unknown_jobs_never_invent_a_name() {
        let text = base_name(42);
        assert_eq!(text, "职业 42");
        for job in 0..9 {
            assert!(!text.contains(base_name(job).as_str()), "{text} 混进了 {job} 号职业名");
        }
        assert_eq!(family(42), "");
        assert!(allowed(42).is_empty());
        assert_eq!(index_of_family("at mage"), Some(8));
        assert_eq!(index_of_family("demonic swordman"), None);
    }
}
