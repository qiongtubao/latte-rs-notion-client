//! 数据模型：时间碎片事件、消费记录、项目及枚举。

use serde::{Deserialize, Serialize};

/// 时间碎片标签
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tag {
    Work,
    Sport,
    Life,
    Study,
    Reading,
}

impl Tag {
    pub const ALL: [Tag; 5] = [Tag::Work, Tag::Sport, Tag::Life, Tag::Study, Tag::Reading];

    pub fn label(self) -> &'static str {
        match self {
            Tag::Work => "工作",
            Tag::Sport => "运动",
            Tag::Life => "生活",
            Tag::Study => "学习",
            Tag::Reading => "看书",
        }
    }

    pub fn from_label(s: &str) -> Option<Tag> {
        Self::ALL.iter().copied().find(|t| t.label() == s)
    }
}

/// 消费分类
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Food,
    Transport,
    Shopping,
    Fun,
    Other,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Food,
        Category::Transport,
        Category::Shopping,
        Category::Fun,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Food => "餐饮",
            Category::Transport => "交通",
            Category::Shopping => "购物",
            Category::Fun => "娱乐",
            Category::Other => "其他",
        }
    }

    pub fn from_label(s: &str) -> Option<Category> {
        Self::ALL.iter().copied().find(|c| c.label() == s)
    }
}

/// 好想法标签
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdeaTag {
    Inspiration,
    Todo,
    Reading,
    Question,
    Other,
}

impl IdeaTag {
    pub const ALL: [IdeaTag; 5] = [
        IdeaTag::Inspiration,
        IdeaTag::Todo,
        IdeaTag::Reading,
        IdeaTag::Question,
        IdeaTag::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            IdeaTag::Inspiration => "灵感",
            IdeaTag::Todo => "待办",
            IdeaTag::Reading => "读书",
            IdeaTag::Question => "问题",
            IdeaTag::Other => "其他",
        }
    }

    pub fn from_label(s: &str) -> Option<IdeaTag> {
        Self::ALL.iter().copied().find(|t| t.label() == s)
    }
}

/// 任务优先级
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskPriority {
    High,
    Mid,
    Low,
}

impl TaskPriority {
    /// 按重要程度从高到低
    pub const ALL: [TaskPriority; 3] = [TaskPriority::High, TaskPriority::Mid, TaskPriority::Low];

    pub fn label(self) -> &'static str {
        match self {
            TaskPriority::High => "高",
            TaskPriority::Mid => "中",
            TaskPriority::Low => "低",
        }
    }

    pub fn from_label(s: &str) -> Option<TaskPriority> {
        Self::ALL.into_iter().find(|p| p.label() == s)
    }
}

/// 项目状态（看板列，按流程顺序）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProjectStatus {
    Backlog,
    Todo,
    Doing,
    Done,
    Paused,
}

impl ProjectStatus {
    pub const ALL: [ProjectStatus; 5] = [
        ProjectStatus::Backlog,
        ProjectStatus::Todo,
        ProjectStatus::Doing,
        ProjectStatus::Done,
        ProjectStatus::Paused,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProjectStatus::Backlog => "暂存",
            ProjectStatus::Todo => "待办",
            ProjectStatus::Doing => "进行中",
            ProjectStatus::Done => "已完成",
            ProjectStatus::Paused => "暂停",
        }
    }

    pub fn next(self) -> ProjectStatus {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn from_label(s: &str) -> Option<ProjectStatus> {
        Self::ALL.into_iter().find(|st| st.label() == s)
    }
}

/// 外部记录（/api/ext，按命名空间隔离，本地 id 由调用方指定）
#[derive(Clone, Debug, PartialEq)]
pub struct ExtRecord {
    pub ns: String,
    pub id: String,
    pub title: String,
    /// 任意 JSON object 的序列化文本
    pub props_json: String,
    pub content_md: String,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 知识库条目类型：目录 / 文档
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoteKind {
    Dir,
    Doc,
}

impl NoteKind {
    pub const ALL: [NoteKind; 2] = [NoteKind::Dir, NoteKind::Doc];

    pub fn label(self) -> &'static str {
        match self {
            NoteKind::Dir => "dir",
            NoteKind::Doc => "doc",
        }
    }

    pub fn from_label(s: &str) -> Option<NoteKind> {
        Self::ALL.into_iter().find(|k| k.label() == s)
    }
}

/// 知识库条目（parent_id 为空表示根节点，无限层级）
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: NoteKind,
    pub title: String,
    pub content_md: String,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 时间碎片事件（end_ts 为空表示进行中）
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub start_ts: i64,
    pub end_ts: Option<i64>,
    pub content: String,
    pub tag: Tag,
    /// 是否提醒（纯本地字段，不同步 Notion）
    pub remind: bool,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 消费记录（金额以「分」存储）
#[derive(Clone, Debug, PartialEq)]
pub struct Expense {
    pub id: String,
    pub item: String,
    pub amount_cents: i64,
    pub ts: i64,
    pub category: Category,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 好想法卡片
#[derive(Clone, Debug, PartialEq)]
pub struct Idea {
    pub id: String,
    pub content: String,
    pub tag: IdeaTag,
    pub pinned: bool,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 今日任务
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    /// 本地日期（YYYY-MM-DD）
    pub date: String,
    pub title: String,
    pub priority: TaskPriority,
    pub done: bool,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}

/// 项目
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub status: ProjectStatus,
    pub start_ts: Option<i64>,
    pub deadline_ts: Option<i64>,
    pub note: String,
    pub notion_page_id: Option<String>,
    pub dirty: bool,
    pub deleted: bool,
}
