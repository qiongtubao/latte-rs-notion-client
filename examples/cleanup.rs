//! 一次性清理：删除 Notion 中未被当前配置引用、且为空的 Latte 数据库。
//!
//! 运行方式：
//!     cargo run --example cleanup -- --dry-run   # 先预览会被删除的空库
//!     cargo run --example cleanup                # 确认后真正删除
//!
//! 只会删除标题为 Latte 标准库名（时间碎片/金钱记录/项目管理/📚 知识库/好想法/今日任务/✅ 每日打卡）
//! 且里面没有任何 page 行的数据库；当前 config 里正在使用的那几个库会被保护，不会删除。

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let dry_run = std::env::args().nth(1).as_deref() == Some("--dry-run");

    let Some(cfg) = latte::config::load()? else {
        println!("未找到 ~/.config/latte/config.toml，无需清理");
        return Ok(());
    };

    if cfg.token.is_empty() {
        println!("配置文件中没有 Notion token，无法清理");
        return Ok(());
    }

    let client = latte::notion::NotionClient::new(&cfg.token);
    let found = client
        .cleanup_empty_duplicate_databases(&cfg, dry_run)
        .await?;

    if found.is_empty() {
        println!("没有发现可删除的空数据库");
        return Ok(());
    }

    if dry_run {
        println!(
            "以下 {} 个空数据库将被删除（当前配置正在使用的库已排除）：",
            found.len()
        );
        for (id, title) in &found {
            println!("- {} ({})", title, id);
        }
        println!("\n去掉 --dry-run 重新运行即可真正删除");
        return Ok(());
    }

    println!("已删除 {} 个空数据库：", found.len());
    for (id, title) in found {
        println!("- {} ({})", title, id);
    }
    Ok(())
}
