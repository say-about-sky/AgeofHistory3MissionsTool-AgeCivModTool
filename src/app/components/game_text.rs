//! 游戏文本（§ 颜色代码）解析与预览。
//!
//! 游戏脚本中的文本字段（`mission_desc`、`desc`、`title`、`name` 等）支持
//! `§` 颜色代码：`§` 后跟一个字符切换其后续文本的颜色，代码字符本身不显示；
//! `§0` 表示恢复默认颜色。文本中的字面 `\n`（反斜杠 + n）表示换行。
//!
//! 色值来自《暮色黄昏：世界大战 0.25.1》游戏源码：`assets/rainfall/polaris_core.json`
//! 的 `textColors` 表，由 `team.rainfall.fontFix.text.TextColorRenderer.getColor()`
//! 渲染（alpha 恒为 FF；全表小写，大写代码无效）。

use dioxus::prelude::*;

/// `§x` 颜色代码 → CSS 颜色；返回 `None` 表示不是颜色代码（原样显示）。
pub fn color_code_css(code: char) -> Option<&'static str> {
	Some(match code {
		'1' => "#000000", // 黑
		'2' => "#4b4b4b", // 深灰
		'3' => "#cbcc02", // 橄榄黄
		'4' => "#d27475", // 玫瑰
		'5' => "#e2990e", // 橙金
		'a' => "#920000", // 暗红
		'b' => "#1c559f", // 蓝
		'c' => "#1eabb5", // 青
		'd' => "#2f8a87", // 灰青
		'f' => "#ef805c", // 鲑橙
		'g' => "#1c933a", // 绿
		'h' => "#9d6637", // 棕黄
		'i' => "#448a45", // 中绿
		'j' => "#c3b091", // 沙色
		'm' => "#2b389e", // 靛蓝
		'n' => "#7c3000", // 深棕
		'o' => "#00aeef", // 天蓝
		'p' => "#522371", // 深紫
		'q' => "#a91b4f", // 玫红
		'r' => "#e03c30", // 红
		's' => "#795234", // 棕
		'y' => "#cdac25", // 暗金
		'z' => "#916dd9", // 淡紫
		_ => return None,
	})
}

/// 文本是否包含颜色代码或 `\n` 转义（决定是否需要显示预览）。
pub fn has_game_codes(text: &str) -> bool {
	text.contains('§') || text.contains("\\n")
}

/// `§x` 代码的渲染动作（依据游戏源码 `TextColorRenderer.getColor()`）。
enum CodeAction {
	/// 切换为颜色表中的颜色。
	Color(&'static str),
	/// 不产生颜色，但代码字符被隐藏：`§0`（基础色）、`§_`（阴影开关）、`§!`。
	Reset,
}

/// 分析 `§` 后面的一个字符；`None` 表示不是已知代码（`§` 原样显示）。
fn code_action(code: char) -> Option<CodeAction> {
	match code {
		// 源码中这三者在查表前直接返回基础色。
		'0' | '_' | '!' => Some(CodeAction::Reset),
		_ => color_code_css(code).map(CodeAction::Color),
	}
}

/// 把游戏文本解析为若干 `(颜色, 片段)`：
/// - 颜色代码被隐藏并切换后续颜色；`§0`/`§_`/`§!` 隐藏并恢复基础色（`None`）；
/// - 未知的 `§` 代码原样保留（如大写代码、`§{...}` 占位符、`§[X]` 行内图片）；
/// - 字面 `\n` 转换为真正的换行符。
pub fn parse_game_text(text: &str) -> Vec<(Option<&'static str>, String)> {
	let mut segments: Vec<(Option<&'static str>, String)> = Vec::new();
	let mut color: Option<&'static str> = None;
	let mut buffer = String::new();
	let mut chars = text.chars().peekable();
	while let Some(ch) = chars.next() {
		match ch {
			'§' => match chars.peek().copied().and_then(code_action) {
				Some(action) => {
					if !buffer.is_empty() {
						segments.push((color, std::mem::take(&mut buffer)));
					}
					chars.next();
					color = match action {
						CodeAction::Color(css) => Some(css),
						CodeAction::Reset => None,
					};
				}
				None => buffer.push('§'),
			},
			'\\' => {
				if chars.peek() == Some(&'n') {
					chars.next();
					buffer.push('\n');
				} else {
					buffer.push('\\');
				}
			}
			_ => buffer.push(ch),
		}
	}
	if !buffer.is_empty() {
		segments.push((color, buffer));
	}
	segments
}

/// 游戏文本预览：隐藏 `§` 代码、按颜色渲染各片段，`\n` 显示为换行。
#[component]
pub fn GameTextPreview(text: String) -> Element {
	let segments = parse_game_text(&text);
	rsx! {
        div { class: "game-text-preview",
            for (color , part) in segments {
                if let Some(css) = color {
                    span { style: "color: {css};", "{part}" }
                } else {
                    span { "{part}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_color_switch_and_reset() {
		let segments = parse_game_text("§r红§0白");
		assert_eq!(segments.len(), 2);
		assert_eq!(segments[0].0, Some("#e03c30"));
		assert_eq!(segments[0].1, "红");
		assert_eq!(segments[1].0, None);
		assert_eq!(segments[1].1, "白");
	}

	#[test]
	fn hides_reset_marks_and_keeps_unknown() {
		let segments = parse_game_text("§!a§_b §Y");
		assert_eq!(segments.len(), 2);
		assert_eq!(segments[0].0, None);
		assert_eq!(segments[0].1, "a");
		assert_eq!(segments[1].0, None);
		assert_eq!(segments[1].1, "b §Y");
	}

	#[test]
	fn converts_escaped_newlines() {
		let segments = parse_game_text("第一行\\n§y第二行");
		assert_eq!(segments.len(), 2);
		assert_eq!(segments[0].0, None);
		assert_eq!(segments[0].1, "第一行\n");
		assert_eq!(segments[1].0, Some("#cdac25"));
		assert_eq!(segments[1].1, "第二行");
	}
}
