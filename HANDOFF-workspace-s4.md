# Handoff: workspace Codex 1:1 — 返工轮（#TASK-2539 FAIL: 10M+2L）

Worktree: .claude/worktrees/workspace-codex-1to1；设计 docs/design/desktop-workspace-management.md v2
已 commit：fc65ab8d8(S1) a4f5b129f(S2) c2b241314(S3) 91d8a2385(S4) 8475a2840(S5)
评审原文：garyx task get '#TASK-2539'（12 条 findings 逐条修，修完 thread send 唤醒 #TASK-2539 复审——注意：返工完成后用 garyx thread send task '#TASK-2539' 送复审即停轮）

## 修法（逐条，已想清）
1. **投影列改普通列+写路径派生（杀 SQL/Rust sanitizer 漂移）**：thread_meta.root_workspace_path
   从 VIRTUAL 生成列改普通 TEXT 列；schema.rs 删 THREAD_META_ROOT_WORKSPACE_PATH_EXPR 与
   generated ensure（改普通 ALTER TEXT）；meta.rs ThreadMetaDraft/Record/upsert SQL 加字段；
   thread_meta_projection.rs 派生时调 workspace_mode::thread_root_workspace_path（唯一真相源）；
   migrations.rs 新增 import-generation-aware 版本化 cutover `thread_meta_root_workspace_v1`
   （仿 RECENT_TASK_THREAD_KIND 模式：读每行 workspace_dir/worktree_json/thread_id 用 Rust 函数
   算后 UPDATE，记 projection_states marker + based_on_import_generation）；
   thread_meta_schema_v2 重建 DDL 改普通列，INSERT SELECT 直接 SELECT root_workspace_path。
   设计文档 §5.2 同步（普通列+同事务写派生+版本化 cutover）。
2. **workspace_origin 持久化**：thread record data 增 `workspace_origin` 字段：
   routes/threads.rs implicit ensure 分支写 "implicit"；显式 workspace 创建路径写 "explicit"
   （create_thread_for_agent_reference 落 workspace_dir 处 + prepare.rs 739/803 implicit 分支）。
   投影列 thread_meta.workspace_origin（普通 TEXT，写路径派生：data 有值用值，缺失用
   thread_workspace_origin 推断）；summary 读投影/record 值而非重算；cutover 与 #1 同一迁移填。
   thread_workspace_origin 函数降级为"历史行推断 fallback"。设计 §5.5 措辞校准。
3. **删客户端第二真相**：thread-model.ts visibleWorkspaceList 删 worktreeKeys/isManagedWorktreePath
   启发式过滤（服务端 root 列表即真相）；分组键改 `thread.rootWorkspacePath ?? null`（不回退
   workspacePath——同仓同发无旧网关兼容，feedback_no_legacy_gateway_compat）。
4. **draft 默认=服务端全序首行 + 惰性一次解析**：resolveDefaultDraftWorkspace 改
   candidates[0]（删 activity 扫描），draft-workspace-selection.test.mjs 同步；AppShell 加
   effect：newThreadDraftActive && selection===null && workspaces.length>0 → 解析一次
   （初始 route/冷启动场景补解析）；removal 重解析保留（设计 §4.2 补一句 sanction）。
5. **explicit footer 显示路径**：ThreadPage composerContext 加分支：
   sentWorkspaceOrigin==='explicit' && !composerWorkspaceMode → 路径 pill
   （abbreviatePath(activeThreadSummary.workspacePath, gatewayHome)，CodexChipProjectIcon）。
6. **side chat=只读继承显示**（fork 语义，设计 §4.2 改准确）：SideChatPanel 传源 thread 的
   workspacePath/origin，ThreadPage side-chat 变体渲染只读 pill（不渲染可选 chip——fork 不可选）。
   设计文档改："side chats inherit the fork source's workspace; the composer shows it read-only"。
7. **picker 统一+Web catalog**：抽 WorkspacePickerContent 共享组件（搜索+服务端序列表+
   check+footer Add/No workspace，Codex icons/文案），WorkspaceComposerChip 的 Popover 与
   WorkspaceSelectDialog 的 Dialog 共用；adapter 加 listCatalog()（Electron=fetch state 里的
   workspaces?直接 window.garyxDesktop.getState? 用 requestDesktopState 拿 state.workspaces；
   Web=fetchWorkspaceCatalog）；WorkspacePathPicker 无 workspaces prop 时经 adapter 拉取；
   WebSettingsPage picker 因此有列表。
8. **epoch 完备**：AppShell workspaceEpoch 作 key 挂到 chip/picker/add-dialog 组件（remount 关
   内部 open state）；add/pin/rename handler await 前捕获 epoch、await 后 epochRef 不符则丢弃
   结果（不 setState/不 sync route/不 close dialog resolve）；workspaceGitStatusCache 在 epoch
   effect 里清空（查 cache 有无 clear，没有加）。
9. **fidelity**：①workspace-rails.css 1135-1145 的 32px 覆盖删/让位（rail 已退役该段可能整段死
   代码，确认后删）；②CodexProjectIcon 内容=三点（提取错），用 picker__itemProject.svg 的
   path 重生成（rg 确认与 projectActions 不同）；③composer.css 的 .workspace-picker-search 与
   dialogs.css 撞名→composer 侧改名 workspace-chip-picker-*；popover 宽 300→260px；
   ④Add dialog：DialogContent 加专属 class（宽 520px、radius 25px）+ CTA 文案 'Save'→
   'Add workspace'（i18n）。对照 extract-create-dialog.json/style-tokens.json。
10. **键盘可达**：WorkspaceThreadSidebar 全部 tabIndex={-1} 删除（button 原生 Tab+Enter）。
11. **permission_denied 回归测试**：workspaces.rs tests 加 cfg(unix) chmod 0o000 目录探针
    （tempfile+std::fs::set_permissions，测后恢复 0o755 保证 tempdir 可清）。
12. **i18n**：加 'New thread': '新建线程'（大小写区别于已有 'New Thread'）。

## 验证基线
cargo test -p garyx-gateway --lib（947+新增）；desktop npm run test:unit（908+）+tsc。
修完全绿 → commit（可 1-2 个主题 commit）→ garyx thread send task '#TASK-2539' 附修复清单
+commit hash 请求复审 → 送审即停。PASS 后流程见上一版 handoff（rebase origin/main→合入→push→
清 worktree→capsule）。

## 坑
zsh 别 echo ===；python replace 必 assert；HANDOFF 不入库（git add -A 后 git reset 它）。

## 复审轮 2 进行中（f8912e31f 已送 #TASK-2539）
reviewer 边跑边实锤的两个新问题（等完整报告后一起修）：
A. ensure_thread_meta_membership_columns 的 DROP COLUMN root_workspace_path 会被上一
   revision 建的 idx_thread_meta_root_workspace 索引挡住（SQLite index 依赖）→ 先
   DROP INDEX IF EXISTS 再 DROP COLUMN（ensure_thread_meta_indexes 稍后重建普通列索引）。
B. 显式创建路径（HTTP create/task 创建带 workspace_dir、fork、resume）没 stamp
   workspace_origin。修：create_thread_for_agent_reference 落 workspace_dir 时写
   "explicit"；fork 继承 source record 的 effective origin；resume 写 "explicit"。
