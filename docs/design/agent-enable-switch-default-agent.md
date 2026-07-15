# Agent 启用开关 + 全局默认 Agent

日期：2026-07-16 · 状态：设计评审中（v16，回应 #TASK-2320 第十五轮：协调器取消安全 + raw API 收窄）

## 1. 需求与产品裁决（用户原话为准）

1. agent 增加启用/停用开关：**不开启的 agent 不可选**（各端 picker）、**CLI 也不可派活**。
2. **list 时明确展示可用性**（CLI、桌面、iOS 的管理列表）。
3. 多渠道搞定（gateway / CLI / desktop / mobile / bot 渠道 / automation）。
4. 支持**设置默认 agent**：默认 agent = 每次新建线程默认选中的那个。
5. **停用语义裁决（2026-07-15 用户明示）**：停用时，该 agent 名下**已开始/已绑定它的线程全部不受影响，可以继续调**；只是**不能新建线程、不能新建 task**。

**统一判据：enabled 只约束「产生新绑定」的动作；纯配置、既有绑定的使用/派生一律不受约束。模式/形态转换若使一个 agent 将被用于未来的新线程，即视为新绑定。**

## 2. 现状事实（六轮评审核验后版本，实现时逐条再核对）

- `CustomAgentProfile`（`custom_agent.rs:9`）无 enabled；`CustomAgentStore` 持久化 `custom-agents.json` 裸 map（load `:69`；persist `:289-297` 过滤 built-in）；`delete_agent`（`:273-283`）可删任意 custom。**store 变更顺序 = 先改内存再写盘**（upsert `:254`、delete `:281`），`atomic_write` 只防文件撕裂（`atomic_write.rs:6`），写失败不回滚内存。
- `resolve_agent_reference`（`agent_reference.rs:39`）被已有线程续跑消费（`prepare.rs:173`、`lifecycle.rs:422`、`run_management.rs:383`）——enabled 检查不能放这里。
- 默认解析现状：requested → current → `"claude"`（`agent_identity.rs:13,15`）；automation update 依赖 current（`automation.rs:503,948,967`）。
- **外部 metadata 通道**：`/api/chat/start` 与 WS start 接受任意 `metadata`（`contracts.rs:38`），外部边界只剥 `provider_env`（`prepare.rs:120`），agent metadata 用 `entry.or_insert` 客户端值优先（`prepare.rs:196`）；bridge 直接消费 `metadata.agent_id`（`run_management.rs:80,403`）。**但（六轮核验）该合并组不只含身份**：`agent_runtime_metadata`（`agent_reference.rs:70`）同时返回 `model`/reasoning/tier/system prompt，且现有优先级契约 = **请求 > 线程模型单元 > agent 默认**（`provider.rs:150`，回归测试 `prepare/tests.rs:289`；创建时 typed `body.model` 写线程模型单元 `routes.rs:2727`，续跑先合并线程单元 `prepare.rs:169`）——整组改 insert 会破坏合法模型优先级。**`CreateThreadBody.metadata`（`routes.rs:901`）不经过 `prepare.rs:120`**，走 `routes.rs:2727` → creator，是独立边界；中央 snapshot 合并保留调用方 `agent_id`（`agent_identity.rs:50`）。
- **claude 物化/哨兵点全清单**：task（`garyx-router/tasks.rs:423`）、cron（`cron.rs:1227-1229`；`automation.rs:494-500`；None/空串/纯空白均视 claude）、CLI channels（`channels.rs:1147`）；channel account = 四 typed + `ApiAccount` 共五类，serde 默认 claude（`config.rs:264,325`）、`ApiAccount.agent_id` 必填（`:443`）、onboard 写 claude（`onboard.rs:66`）、router 把账号值当 requested（`garyx-router/threads.rs:839`）；desktop `gateway-settings.ts:259`/`channel-setup.ts:101` 补 claude、序列化器删显式 claude（`gateway-settings.ts:146,179`）；Web shell 物化（`use-web-settings-state.ts:41`、`WebSettingsPage.tsx:387,609`；`build:web` 在 `package.json:17`）；bridge plugin None 解 claude（`lifecycle.rs:190`；`:168` 是 API account 注释）；bot 读 API None 输出空串（`routes.rs:3376,3448`）；desktop route claude 哨兵（`desktop-route.ts:185,223`）；side-chat fork 物化（`side-chat-ops.ts:84`）。
- **automation 现状**：`AutomationSummary.agent_id` 必填（`automation.rs:101`）；target-existing 不创建线程、运行读目标线程（`automation.rs:858-874` 跳过校验；`cron.rs:1256-1269` 仅 generated 进中央创建；**运行取目标线程 `cron.rs:1938`，而摘要展示 job 字段 `automation.rs:585`**）；**target 创建省略 agent 时服务端/桌面会物化 claude 并显式存入 job**（`automation.rs:867,791`、`useAutomationController.ts:260`）——"目标线程 Codex、job 显式 Claude"是现存合法数据；**update 清除目标后用 current 解析**（`automation.rs:948,967`）——target→generated 转换可保留 disabled current；`CronService::load` 磁盘 → config jobs 覆盖 → 持久化（`cron.rs:748-781`）；`AutomationPrompt` 还含 Log job（`cron.rs:1531`）；`InternalDispatch` 的 None 合法（`schedule_followup.rs:172-187`、`quota_resend.rs:186-207`）；desktop 编辑恒发显式 agentId（`useAutomationController.ts:38,265`）、disabled 重插可选（`AutomationDialog.tsx:376`）；iOS 静默换绑（`GaryxMobileAutomationViews.swift:533,558`）、解码补 claude（`GaryxGatewayAutomationModels.swift:77`）。**wire 层（六轮核验）全链要求非空 `agent_id: String`**：服务端 `automation.rs:99`、desktop mapper 强制非空（`automations.ts:178`）、iOS 必填 String 缺失补 claude（`GaryxGatewayAutomationModels.swift:8`）、iOS cache 必填（`GaryxMobileCatalogCache.swift:206`）——null/空串会炸契约，哨兵串会伪造身份。**legacy 无绑定线程可在运行期经 task assign 获得绑定**（`gateway/tasks.rs:825`）——启动时缓存的派生值会过期。
- **bridge 状态面**：topology 与 agent profiles **不同锁**（`state.rs:22`），`replace_agent_profiles` 只更新一张表（`multi_provider.rs:118`），API 随后独立 reload topology（`api.rs:2841`）——两阶段有乱序/半应用窗口。**配置独立热更新是生产路径**（`app_state.rs:283`、`api.rs:2017`、`gateway.rs:232`；bridge 测试 `multi_provider/tests.rs:4636` 明确要求不动 agent store 即热更模型）——bridge 更新的版本排序不能只挂在 agent store 计数上。**`get_or_create_provider` 会在 topology 发布前原地改共享 provider 默认值**（`lifecycle.rs:323`）——"全旧或全新"需要显式 staging/swap 边界。
- task 工作区：中央创建应用 `default_workspace_dir`（`agent_identity.rs:104`）而 TaskService 绕过（`gateway/tasks.rs:199,867`）。
- 新建入口穷尽（五轮确认）：中央 creator 直连 = HTTP Fresh/Fork/Recover（`routes.rs:2812`）+ generated cron（`cron.rs:1269`）+ `GatewayThreadCreator`（`agent_identity.rs:158`，生产强制注入 `app_bootstrap.rs:325`，覆盖 bot 入站 / `/api/chat/start` / `/newthread` / task 通知首建）；唯一 raw 绕行 = `TaskService::create_task`（`tasks.rs:440`，五 agent 来源 `:412-426`）。router 失败现状：Claude fallback（`threading/threads.rs:269`）、虚构 key（`:290`）、planning 无错误通道、`/newthread` 压扁文案。
- bot 读 API 客户端：desktop `DesktopBotConsoleSummary`/mapper 不保留 agent_id；iOS bot models 只解析 agentId；Web settings 无全局 effective 数据通道。
- 管理面快捷动作：iOS Chat/Use（`GaryxMobileAgentsViews.swift:1419`）；desktop Chat（`AgentsHubPanel.tsx:597`、`AgentFormDialog.tsx:680`）。客户端顶层字段：desktop `agents.ts:200`、iOS `GaryxGatewayClient.swift:478` 丢弃；desktop `pendingAgentId`（`AppShell.tsx:652`）与 iOS 选中态（`GaryxMobileModel.swift:299`）必填 string。
- **直达 bridge 旁路（九轮核验，三条）**：① `/api/chat/start` 显式 `threadId` 只查格式与 archive（`prepare.rs:473`；`is_thread_key` 仅前缀判断 `thread_logs.rs:110`），记录不存在时 `persist_explicit_api_thread_binding` 仅返回 false 不报错，仍进 `chat.rs:375` `start_agent_run`，bridge 找不到 metadata 按旧 route/default 选 provider 并在 `run_management.rs:615` 首写 `thread_affinity`——**伪造 `thread::任意值` 可绕过 creator/gate 派活**；② thread-less cron：`AutomationPrompt + AgentTurn` 无 workspace_dir/thread/target 跳过 `cron.rs:1269` 中央创建（`:506` 只有带 workspace_dir 的 generated 才进），在 `:1994` 生成 `cron::<job_id>` 伪 thread、`:2162` 直调 bridge，且 `:2032-2069` 构造的 metadata **不应用 `job.agent_id`**；`CronJobConfig` 三字段全可选（`config.rs:988`）载入不拒，无 target 的 `SystemEvent` 共享该直发分支；③ `garyx tool image`（`tool.rs:58`）/`/api/tools/image`（`tool_image.rs:178`）创建全新 `tool::image::*` runtime id 直达 bridge，显式 Codex provider，无 agent 绑定。
- **admission 封闭性现状（十一轮核验）**：`AgentRunRequest` 公开构造、字段全公开可克隆（`provider.rs:88-137`）；`AgentDispatcher::dispatch` 收裸 request（`contracts.rs:11-18`）；`start_agent_run` 公开裸入口（`run_management.rs:438-454`，trait 转发 `multi_provider.rs:227-236`）；`ThreadStore::get` 返回可任意构造的 `serde_json::Value`（`store.rs:27-34`），`ThreadRecord` 为 `Default` 且字段全公开（`thread_record.rs:110-127`）——"持有 record 即证明"不成立；bridge 在 `requested_provider` 之外仍优先消费 `metadata.agent_id`（`run_management.rs:400-410`）——`tool::* + agent_id` 可混入 agent。**TOCTOU**：archive/delete（`routes.rs:3108,3137-3186`）不消费 bridge 的 thread guard（`run_management.rs:455-462` 才取得），active run/affinity 更晚注册（`:605-619`）——record 校验后仍可被并发 archive。cron 在线 mutation 路径：`CronService::update`（`cron.rs:947-975`，automation handler `automation.rs:1030` 调用）、`from_config/add/upsert`（`cron.rs:142-165,911-943`）。
- **run 生命周期细部（十二轮核验）**：`MultiProviderBridge` 由 `lib.rs:13` 公开导出，还有公开入口 **`run_inline_streaming`**（`run_management.rs:1203-1211`，收裸 thread_id、`:1228-1234` 无 record 仍继续、`:1263-1277` 注册启动；当前仅测试调用，但类型不变量因它不成立）；archive 的 active 判断读 **transcript `run_state`**（`routes.rs:2977-3005`）而非 bridge `run_index`；transcript `run_start` 由异步 persistence worker 提交（spawn `run_management.rs:674-690`，提交 `persistence_worker.rs:387-406,421+`，启动方不等首提交）——`run_index` 注册与 `run_start` 落盘之间有窗口；同线程已有 streaming run 时 `start_agent_run` 直接返回 **`QueuedToActiveRun`**（`run_management.rs:501-532`）；archive 在 `routes.rs:3108,3137-3186`（对 active run 返回冲突 `:3137-3139`），**独立 DELETE 在 `:3198-3259`（调 `hard_delete_thread_record(...,true)` `:3258`，先 `abort_thread_runs` 再删 `:1149-1152`）、底层 hard delete 在 `:1136-1173`**；`abort_run` 只 `task.abort()` 不等 JoinHandle（`run_management.rs:1838-1847`）；**router 另有删除原语 `threads.rs:660-681`（cron `cron.rs:1300,1374` 使用），创建回滚删除在 `routes.rs:2852,2865`**。
- **公开面补遗（十三轮核验）**：`lib.rs:11` 公开导出 `run_graph`，`RunGraphState` 字段/构造器公开（`run_graph.rs:81-100`）、`execute_agent_run` 公开（`:371-392`，`:217-236` 直调 provider）、`ProviderRuntime::run_streaming` 公开（`provider_trait.rs:107-126`）且三种 provider 构造器公开。
- **公开面补遗 2（十四轮核验）**：`MultiProviderBridge::get_provider` 公开返回 `Arc<dyn ProviderRuntime>`（`topology.rs:46-54`），gateway 生产代码已持有 live handle（`dashboard.rs:89`、`mcp/helpers.rs:69`）——public-API deny-list 挡不住"既有 handle 上新增一行 `run_streaming` 调用"。**archive 顺序**：检查 active（`routes.rs:3137-3139`）→ 逐个 detach endpoint（`:3144-3164`）→ 最后提交 tombstone（`:3175-3183`）——lease 检查若只落最终原语，中途已产生 endpoint 副作用。**（v16 修正十五轮核验）`legacy_boot_import.rs:823` 是 `#[cfg(test)]` 测试桩（`:522,799-824`），不是生产 boot 路径——生产 `archive_thread_record` 调用仅 `routes.rs:3117,3179`**；`garyx_db` 是公开模块（`lib.rs:19`），`archive_thread_record`（`garyx_db/mod.rs:650`）与 `delete_thread_record_with_projections`（`:2036`）均 public——raw 破坏性 API 未封闭。`run_blocking`（`garyx_db/mod.rs:523-531`）直接 await `spawn_blocking`，Tokio blocking task 一旦开始不可 abort——HTTP future 取消不会停下已启动的 DB 操作。
- **persistence 失败语义（十三轮核验）**：首次 transcript append 失败只记 warning 继续（`persistence.rs:1192-1213`），thread record 写失败同（`:1279-1280`）；provider run 独立继续（`run_management.rs:828-829`），worker 在 provider 完成后才被等待（`:850-864`）——**首提交失败 ≠ run 结束**。
- **cron/automation validation 现状（十轮核验）**：`CronJob`（`cron.rs:103-138`）与 `AutomationSummary`（`automation.rs:101-120`）均无 invalid/error 字段；`is_automation_job`（`automation.rs:657-670`）要求 AgentTurn 有 workspace 或 target——恰好把 thread-less invalid job 从 automation 面过滤掉（不可见不可修）；放宽后 `automation.rs:513-525` 又会因缺 workspace 令整个 summary 转换失败；`/api/cron/jobs` 不输出校验状态（`api.rs:1413-1430`）。
- 验证：desktop `test:unit`（无 TS 编译）+ `build:ui` + `build:web`；iOS App-target 须 `xcodebuild`。

## 3. 设计

### 3.1 数据模型、持久化、共享解析层与 bridge 一致性合同

- `CustomAgentProfile` 加 `enabled: bool`，serde 默认 true。
- `custom-agents.json` 升 versioned envelope（判别：顶层数字 `version` + 对象 `agents`；否则 legacy 裸 map，load 迁移、persist 恒写 v2）：`{ "version": 2, "agents": {...}, "disabled_builtin_ids": [...], "default_agent_id": "codex" }`。
- **garyx-models 唯一解析实现**：`AgentAvailabilitySnapshot`（agent 元组含 enabled/standalone/built_in/default_workspace_dir/provider-runtime 字段 + `default_agent_id` + **`agent_state_revision`**）+ 纯函数 `resolve_effective_default` / `ensure_enabled_for_new_binding`。
- **store 变更合同**：所有 mutation（upsert/delete/toggle/set-default）= **clone next state → persist 到磁盘 → 成功后 swap 内存（锁内，`agent_state_revision` 单调递增）**；persist 失败则内存/磁盘全不变、API 明确报错。现行"先改内存后写盘"顺序废弃。
- **bridge 非决策点原则（v9，取代 v6-v8 的发布协议）**：
  - **enabled/default 的一切决策都发生在 gateway/router 侧的决策点**（新绑定 gate、effective default 解析、list 的 effective 值），这些点**同步读 store**（与 mutation 同锁序，天然线性化）。bridge **不消费 enabled/default、不做解析**。
  - **新增不变量（v12：sealed run admission + 原子 run intent，取代入口枚举）**：
    - **sealed 类型边界**：dispatcher/bridge 生产边界改收 **`AdmittedRun::{ThreadBound, ProviderTool}`**——sealed、字段私有、生产代码外不可构造不可克隆；裸 `start_agent_run(AgentRunRequest)` 对生产调用方**不再可见**（`run_management.rs:438-454`、trait 转发 `multi_provider.rs:227-236` 收窄可见性），`AgentDispatcher::dispatch`（`contracts.rs:11-18`）与 `DeferredFanoutAgentDispatcher` 等包装层同步改签名收 `AdmittedRun`。持有 record ≠ 证明（`ThreadRecord` 可 Default 可克隆）——证明来自**只能由 store-backed admission API 返回的 sealed 值**，admission 内部自己 point-read + 校验，**不接受调用方提供的 record**。
    - **`ThreadBound` 的原子 admission = 校验 record + 取得 run lease（v13：完整 lease 生命周期，回应十二轮 #2）**：admission 在 thread store 的同一线性化点完成「record 存在且非 archived」校验并产生 **store-visible、per-request、cancel-safe 的 run lease**。生命周期合同：
      - **`Started`**：lease **原子转为 active lease**（对 archive/delete 同样可见），由 run terminal cleanup 释放——archive 的 active 判断以 **lease 域为准**，不再依赖 transcript `run_state`（`routes.rs:2977-3005` 现状：`run_index` 注册与异步 `run_start` 落盘之间有窗口，`persistence_worker.rs:387-406`——lease 正是为覆盖该窗口）；
      - **`QueuedToActiveRun`**（`run_management.rs:501-532`）：**确认既有 active lease 接管本次输入后**才释放本请求 lease（不释放即泄漏、永久阻塞删除）；
      - **error / future 取消 / panic / drop**：一律不泄漏（cancel-safe guard，drop 即释放）；
      - **同线程并发请求各持独立 lease**，不破坏现有 follow-up 排队语义；
      - **archive（`routes.rs:3108,3137-3186`）、独立 DELETE（`:3198-3259`）、底层 hard delete（`:1136-1173`）全部消费同一 lease 域**：archive/delete 先赢 → admission 返回 typed stale/not-found、bridge 零调用；lease 在持时的行为按下方「删除策略拍板」执行（archive 409、DELETE abort-and-drain），绝无"删除后再启动"。
      existing 来源统一重验：显式 `threadId` 不存在 404（修 `prepare.rs:473`）；bot endpoint 快照 stale → 绕过快照 fresh re-resolve、线程确已不在则按首次接触走 Fresh creator/gate；`cron::` scheduled-reply 特判自然不可达；HTTP/WS 共用 prepare 同一合同；生产构造点 `chat.rs:376`、`cron.rs:2163`、`router/run/execution.rs:174` 全部改经 admission。隐式创建唯一通道 = Fresh creator/gate；真实存在但无 stamp 的 legacy thread 兼容 fallback 守恒。
    - **公开运行入口穷尽（v13/v14/v15）**：`run_inline_streaming`（`run_management.rs:1203-1211`）、**`run_graph::execute_agent_run` 与 `RunGraphState`（`run_graph.rs:81-100,371-392`）一并收窄为 crate 内部**；**`ProviderRuntime::run_streaming`（`provider_trait.rs:107-126`）定性为有意保留的 provider SPI 例外**。因 `get_provider` 公开返回 live handle（`topology.rs:46-54`）且 gateway 已持有（`dashboard.rs:89`、`mcp/helpers.rs:69`），public-API deny-list 挡不住后加调用——**守卫改为 CI 精确 call-site allow-list**：生产源码中 `run_streaming` 的调用点白名单（当前仅 `run_graph.rs:219,236`），新增任意调用点即 CI 失败；接口测试同时覆盖 bridge 公开 API 中能启动 run 的函数集不扩大。
    - **archive/DELETE 唯一行为矩阵（v15，取代先前一切表述）**：
      | 时序 | 行为 |
      |---|---|
      | archive 先赢 | 原子「确认无 lease + 设 `archiving` reservation」（**在任何 endpoint detach 之前**，封 `routes.rs:3144-3164` 中途副作用的 TOCTOU）→ 后续 admission 拒绝 → detach → tombstone；任何 detach/DB 失败路径释放 reservation，**lease 冲突时零 endpoint 副作用** |
      | lease 在持时 archive | **409**，不取消 run |
      | DELETE（先赢或 lease 在持） | 进入 `deleting`（挡新 admission）→ **强制失效全部 request/active lease token**（持 token 的调用方恢复执行时在 bridge 入口前见 token 已失效，零调用）→ abort 并 **drain 等待 task 真正终止 + lease 归零**（等待期间**不持 lease-domain 锁**，防 Drop 释放死锁；现 `abort_run` 只 `task.abort()` 不等 JoinHandle `run_management.rs:1838-1847`，须补等待确认）→ 删除；**不是 409** |
      | persistence 首提交失败 + provider 仍在跑 | archive → 409；DELETE → 触发上行 abort-and-drain（非笼统"被阻"） |
      | DELETE 中途失败（abort/drain/最终 delete 任一步） | **`deleting → live` 失败转移**：记录仍在、`deleting` 状态清除、线程可继续续跑（绝不留永久拒绝态） |
      **lease 约束落在 `ThreadStore` 的 archive/delete 原语层**（单一咽喉）+ reservation 前置原子点；router 删除原语（`threads.rs:660-681`，cron 使用）与创建回滚删除（`routes.rs:2852,2865`）自动覆盖（后者无 lease 即放行）。
      - **协调器所有权与取消安全（v16，回应十五轮 #1）**：archive/DELETE mutation **由 store/coordinator 独立拥有，不挂在 HTTP 请求 future 生命周期上**——handler 把操作交给 coordinator 拥有的执行体后即使自身被取消/drop，reservation/`deleting` 状态**不随请求释放**；blocking DB 操作（`run_blocking` await `spawn_blocking`，开始即不可 abort）**确定完成或确定失败前不得恢复 live**；终态只有两种可观察结果：已删除/已 tombstone，或确定失败后的 live。
      - **raw 破坏性 API 收窄（v16，回应十五轮 #2）**：删除先前"boot-only 豁免"的错误表述（`legacy_boot_import.rs:823` 实为测试桩）；`garyx_db::archive_thread_record`/`delete_thread_record_with_projections` 收窄可见性（`pub(crate)` 或模块私有），HTTP route（`routes.rs:3117,3179`）改走协调原语；测试确需 raw 入口则加**生产 call-site 精确守卫**（同 SPI allow-list 机制）。
    - **`ProviderTool`**（`tool_image.rs:186`、`commands/tool.rs:65`）：sealed 变体只允许 `tool::*` runtime id；**provider 只能来自其 sealed 显式字段，构造时剥除/拒绝绑定身份 metadata（`agent_id` 等 reserved 键）**——封死 `tool::* + metadata.agent_id` 混入 agent 的洞（`run_management.rs:400-410` 现状 metadata 优先）。不创建 agent 绑定、不受 enabled 管理（定性可被用户否决；若否决改纳独立 gate）。
    - **thread-less cron 组合退役**：`AgentTurn` 无 workspace/thread/target、无 target 的 `SystemEvent` → load 标 invalid、派发 typed 拒绝（不进 bridge；validation 合同见 3.3）——不补建 canonical thread，避免静默改变 legacy 运行形态；用户显式改成合法组合即恢复（自然过 Fresh gate）。
  - **plugin/channel account `None` 的默认解析前移到线程创建时**：creator（gateway 侧，store 同步）解析并把 canonical `agent_id` stamp 进 thread；bridge 照现状读 thread 绑定。`lifecycle.rs:190` 的 claude fallback 仅剩对无 stamp 的 legacy 线程生效（既有绑定，行为守恒，不改）。
  - **bridge 传播保持现状**：`replace_agent_profiles` 照旧推送 profiles（内含 enabled 字段但 bridge 不据此决策），仅加单调 `agent_state_revision`、应用端丢弃旧 revision 的推送（防乱序把 profile 元数据顶回旧值——廉价保险，非决策依赖）。**不引入 reconcile 通道、coherent unit、last-published 读面**——那套机制是为"bridge 参与决策"设计的，前提已消失；config/topology 热更（settings PUT / MCP / 文件热更）**完全不动现状语义，本特性零触碰**，provider 实例生命周期与活跃 run affinity 亦不受影响。
  - **cold-start 无新依赖**：agent store 随 boot 同步加载（先于 HTTP 开放），gate/list 同步读 store，不存在"首个 published snapshot"问题。
  - HTTP toggle 无并发 token 与内部 revision 是两回事，互不替代。
- `default_agent_id` 与 agent 同库同锁，不进 `garyx.json`。

### 3.2 API

- `GET /api/custom-agents`：每项 `enabled`；顶层 `default_agent_id`（raw，可 null）+ `effective_default_agent_id`（nullable）；恒 200。
- `PATCH /api/custom-agents/{id}/toggle` `{"enabled": bool}`：幂等 set、无并发 token、custom 推进 `updated_at`，走 3.1 变更合同。
- `PATCH /api/custom-agents/{id}/default`：校验存在 + standalone + enabled；无 unset 端点。
- DELETE 命中 raw default：**同一 mutation 内原子清空 `default_agent_id`**（不拒绝删除）。
- `PUT` `enabled: Option<bool>` 三态：create 缺省 true、update 缺省保留。
- **外部绑定身份字段 reserved（v7：精确键集 + 分边界，回应六轮 #1）**：
  - **reserved key 集精确定义且共享为常量**（garyx-models 单一来源）：仅**绑定身份/敏感键**——`agent_id`、`requested_provider_type`、`provider_env`（已有）。**`model`、reasoning、tier、system prompt 不在 reserved 集**，保持现有 fill-only（`or_insert`）语义与「请求 > 线程模型单元 > agent 默认」优先级契约（`provider.rs:150`）不变。
  - **各真实边界分别清剿**（不存在"同一咽喉"）：HTTP/WS chat 边界在 `prepare.rs:120` 现有剥除点扩展 reserved 集；`CreateThreadBody.metadata` 在 `routes.rs:2727` → creator 的入口处独立剥除。
  - 剥除后由服务端以线程 canonical binding 写入身份键：`prepare.rs:196` 与 `agent_identity.rs:50` **仅对 reserved 身份键**改为 canonical 覆盖，其余键合并语义不动。
  - **不提供 metadata 形式的 one-off override**——显式选 agent 的唯一合法通道是 `CreateThreadBody.agent_id` 等 typed 字段（已走新绑定 gate）。
- bot/channel 读 API 继承态：None → `agent_id: null` + `effective_agent_id`；客户端传播点名：desktop `DesktopBotConsoleSummary`/mapper 加 `agentId`+`effectiveAgentId`；iOS `GaryxConfiguredBot` 系列加可选 `effectiveAgentId`；Web settings 以该字段（或 list 顶层 effective）为"跟随全局（当前 X）"数据源。

### 3.3 默认解析链与"未指定"三态合同

解析链（garyx-models 纯函数，保留 current 档）：

```
requested（显式；新建绑定 disabled → AgentDisabled，绝不 fallback）
→ current（既有绑定，enabled 不参与——裁决 5）
→ default_agent_id（enabled 且 standalone）
→ "claude"（enabled 时）
→ 第一个 enabled 的 standalone（built-in 播种序 → custom 按 agent_id 字典序）
→ 无可用（隐式新建 → NoEnabledAgent；effective → null）
```

**channel account `agent_id` 三态合同**（`Some`=显式 override 含 claude；`None`=继承全局）：config 五类 account 统一 `Option<String>`、serde 默认 None、已持久化显式值不迁移；写入方拆物化/拆删除（onboard `onboard.rs:66`、desktop settings/channel-setup、**序列化器禁删显式 claude**、Web shell 修复、CLI）；读取方 router 返回 Option、由 creator 在创建时解析默认并 stamp canonical `agent_id` 进 thread（bridge 不再解析，见 3.1）；Add Bot/账号保存是纯配置全停用照常（标签"跟随全局（当前无可用）"），首次入站新建才被 gate 拒；desktop route null 哨兵；side-chat fork 不物化（`side-chat-ops.ts:84` 源线程无 agentId 传 null）；task/cron/CLI 提前物化拆除。

**automation 的 agent 合同（v6 派生字段 + 模式转换 gate；v7 补 typed wire contract 与实时 current）**：

- **范围限定 automation 的 AgentTurn 类 job**（`AutomationPrompt` 里的 Log job、`InternalDispatch` 一律不受本合同约束）。
- **generated**：agent 恒显式。create 省略 → 按 effective 落显式（全停用 400 NoEnabledAgent）；显式 disabled → 400；merge 后归一化：**仅缺失/空串/纯空白** → 显式 `"claude"`（守恒现行为，幂等，不写回 garyx.json）。
- **target-existing：job 的 agent 是派生值不是配置，wire 层用 typed 状态表达（v7，回应六轮 #3）**。`AutomationSummary` 增加 **`agent_resolution: "resolved" | "follow_thread" | "target_missing"`** + **nullable `effective_agent_id`**：generated 恒 `resolved` + 显式 id；target-existing 恒 `follow_thread`，`effective_agent_id` = 目标线程 canonical binding 的**实时派生值**（线程无绑定时为 null，UI 展示"随线程"占位）；目标线程不存在 → `target_missing`（`effective_agent_id: null`，UI 明确 unavailable 态，该 job 派发失败记可见错误）。**禁用 null/空串塞进现必填 `agent_id` 和任何哨兵串**（desktop mapper `automations.ts:178`、iOS models `GaryxGatewayAutomationModels.swift:8`（删 claude 回填）、iOS cache `GaryxMobileCatalogCache.swift:206`（升版本）、CLI 展示同步适配新字段）。job 存储字段降级为缓存、merge 后每启动从目标线程重算（无论旧值——修复"目标 Codex、job 显式 Claude"脏数据）。
- **target→generated 模式转换 = 新绑定**（统一判据）：服务端在 update 落库前对转换后将用于新线程的 agent **强制 enabled gate**；**current 必须实时读取目标线程当时的绑定**（不得用启动时缓存——无绑定线程可在运行期经 task assign 补绑定，`gateway/tasks.rs:825-847`；已有不同绑定会被 `:776-780` 拒绝）；目标缺失或无绑定时**要求显式选择 enabled agent**（否则 400 明确报错引导重选）；disabled → 400。generated 内只改名（无模式/agent 变化）仍走 current 档放行。
- 客户端编辑合同：`agentChanged` 跟踪、未动省略 `agent_id`；disabled current 只读 missing 不可选、禁静默换绑；generated 新建默认 = effective、选项 enabled-only；target-existing 表单 agent 区展示"随目标线程（当前 X）"只读。
- **invalid job 的 validation wire 合同（v11/v12）**：**唯一纯 validator 函数**，在**最终 load merge、`from_config`/`add`/`upsert`/`update`（`cron.rs:142-165,911-943,947-975`、`automation.rs:1030`）每次 mutation 后重算**，并在 **`run_now` 与 scheduler claim/dispatch 前 fail-closed 重验**（stale validation 不得放行，UI 状态与派发判断同源不漂移）；`CronJob` 的 `validation_error: Option<...>` 为派生字段并 **`#[serde(skip)]`**（不落盘）；`AutomationSummary` 增加 `validation_state: "valid" | "invalid"` + `validation_error`（原因文案，如"缺 canonical target"）。**invalid 的 AgentTurn 必须仍进 `/api/automations`**：放宽 `is_automation_job`（`automation.rs:657-670`）让 thread-less AgentTurn 通过、summary 转换（`automation.rs:513-525`）对缺 workspace 产出 invalid 行而非整体失败——否则不可见不可修，"用户改成合法组合即恢复"无从谈起。无 target 的 `SystemEvent` 不是 automation（历史上也不在 automation UI），留在 `/api/cron/jobs` 并输出校验状态（`api.rs:1413-1430` 加字段）。desktop/iOS automation 列表渲染 invalid 行（错误 badge + 可进编辑修复）；CLI `automation list` 加 validation 列。**测试须含"存在 enabled agent"场景**：thread-less AgentTurn 仍被拒且原因 = 缺 canonical target，证明不是 disabled/全停用 gate 的假绿。

其余：automation update 显式改 disabled → 400；停用当前默认不改写 raw；允许停到一个不剩。

### 3.4 拦截点

**a) 创建意图**：`create_thread_for_agent_reference` 带意图——`Fresh` 拦、`Fork` 拦、`RecoverExistingSession` 豁免。

**b) typed 错误**：`AgentDisabled(agent_id)` + `NoEnabledAgent`。不进 Claude fallback、不虚构 key、planning 补错误通道、`/newthread`/渠道明确文案。覆盖 bot 入站、`/api/chat/start` 无 thread、`/newthread`、task 通知首建。已绑定 bot 返回既有 thread 不拦。

**c) TaskService gate**：router trait `NewTaskAgentGate` → `ResolvedAgentBinding`（canonical id + provider/runtime 快照 + `default_workspace_dir`，落 record 前应用「显式 workspace → agent 默认 → 无」）；`TaskService::new` 必填构造 fail-closed；gateway 注入 store 背书实现；五来源全覆盖；API 层保留友好 400。

**d) assign**：新绑定 disabled 拒绝；既有同-agent task 返工放行。

**e) 明确不拦**（反向验证）：既有线程续聊/steer、thread send 返工、bridge 回填、recovered session、已绑定 bot、target-existing automation（创建与运行）、纯配置动作。**已有 disabled agent 线程在 metadata reserved 字段清剿后必须照常续跑**（canonical binding 本来就是该 agent）。

### 3.5 CLI

- `garyx agent list`（人读注记 + `--json` enabled/raw/effective）；`agent enable|disable <id>`；`agent default [<id>]`。
- `task create --agent` / `thread create --agent` 透传 typed 错误。
- channels picker：共享 loader + 纯解析、滤 `standalone && enabled`、默认建议 = effective、"跟随全局"档位；全停用仍可保存继承档。

### 3.6 桌面端

- AgentsHubPanel：Enabled 列 + 默认标记 + "Set default"；disabled 行隐藏/禁用 Chat、Set default。
- 默认标记四态：raw 正常 = "Default"；raw 停用 = raw 行 muted "Default (inactive)" + effective 行 muted "Acting default"；raw null = effective 行 muted "Default (auto)"。CLI 同语义。
- 全停用空态（限新绑定）：新线程草稿 composer 空态禁发（已有线程 composer 不受影响）；generated automation 新建禁用；target-existing 创建、Add Bot/账号保存照常；管理面恒可用；`pendingAgentId` 改 nullable。
- picker disabled 隐藏；新建草稿默认 = effective；Chat 一次性 override；side-chat fork 不物化。
- AddBotDialog/渠道下拉：默认建议 = effective、排除 disabled、"跟随全局"档位、已绑定 disabled `.missing` 置灰；序列化器保值显式 claude；Web shell 修复。
- bot console 契约加双字段；automation 表单按 3.3（target 模式 agent 区只读"随目标线程"）；route null 哨兵；client 解析顶层双字段。

### 3.7 iOS

- 管理列表：StatusPill + swipe Enable/Disable；disabled 行隐藏 Chat/Use；默认标记四态同 3.6。
- picker：`makeTargets` 滤 `.enabled`；bot picker 默认建议 = effective + "跟随全局"。
- 全停用空态限新线程草稿；选中态改 nullable；已有线程 composer 不受影响。
- 两态拆分：`gatewayDefaultAgentId` 只读缓存（`Use` = set-default → 刷新）+ 草稿一次性 override；移除全部旁路持久写入。
- automation：编辑合同按 3.3（target 模式只读展示派生值）；解码补 claude 删除。
- models：`GaryxAgentSummary`/`GaryxCustomAgentRequest` 加 enabled（三态）；bot models 加 `effectiveAgentId`；`GaryxCachedAgent` 镜像（decodeIfPresent 默认 true）+ snapshot 升 version；client 解析顶层双字段。
- 纯逻辑下沉 Core 配 SwiftPM 测试。

### 3.8 兼容性

- legacy 裸 map → load 迁移写 v2（单向）；channel 显式值不迁移、缺失 None 继承；automation AgentTurn job：generated 缺失/空白归一 claude、target-existing 每启动从目标线程重算（含修复历史显式错值）；均幂等、不写回 garyx.json。
- 旧 catalog 快照不匹配即丢弃；不做旧网关兼容；老客户端 PUT 缺 enabled 由三态保护；服务端 gate 兜底；**老客户端携带的 metadata.agent_id 被 reserved 清剿静默忽略**（行为 = 服务端 canonical 覆盖，无破坏）。

## 4. 测试与验证计划（headless 优先）

- **存储/解析**：envelope 往返、legacy 迁移；解析链全分支 + 稳定序 + 回退；删除 raw default 原子清空 + badge；**持久化失败注入：内存不变、API 报错、无 snapshot 推送**。
- **API**：toggle/set-default/PUT 三态/list 双字段/bot API null+effective；**外部 metadata reserved：enabled A + `metadata.agent_id=B(disabled)` → 线程与 run 均落 A；`CreateThreadBody.metadata.agent_id` 不能制造绑定不一致；已有 disabled B 线程照常续跑；「custom agent 默认模型 + typed 线程模型」在创建与续跑两条路径均保持「请求 > 线程单元 > agent 默认」不被 reserved 清剿破坏**。
- **创建时 stamp 与 bridge 非决策点**：plugin/channel None account 新建线程 stamp 当时 effective default；**默认热切换（claude→codex）后下一个新线程用新默认、既有线程绑定不变**；无 stamp 的 legacy 线程 bridge claude fallback 行为守恒；profile 推送乱序（旧 revision 后到）被丢弃（profiles 与 revision 出自同一次原子 store snapshot）；**config 热更/模型热更现有测试（`multi_provider/tests.rs:4636` 等）全部原样通过——本特性零触碰验证**；store persist 失败注入 → 内存/磁盘全不变 + API 报错。
- **sealed admission 封闭性（v12-v15）**：编译期/接口测试——`AdmittedRun` 在生产模块外不可构造；bridge 公开 API 中能启动 run 的函数集不扩大 + **`run_streaming` 生产调用点 CI 精确 allow-list（当前仅 `run_graph.rs:219,236`，新增即失败）**；**`ProviderTool + metadata.agent_id` 反例**；**TOCTOU barrier：`ThreadBound` 已构造后暂停 → 并发 archive/delete/hard-delete——按 v15 行为矩阵分别断言（archive/delete 先赢→typed stale + bridge 零调用；lease 在持→archive 409 且零 endpoint 副作用、DELETE 走 abort-and-drain 完整序且删除必在 lease 归零后）**；**archive reservation 与 endpoint detach 之间 barrier：lease 冲突时零 detach 副作用**。
- **DELETE 状态机（v15）**：admission 后、dispatch 前暂停 → DELETE 失效 token → 恢复执行 bridge/affinity 零调用；thread delete 注入失败 → 记录仍在、`deleting` 已清除、线程可再续跑；drain 等待不持 lease-domain 锁的 barrier（并发 Drop 释放无死锁）。
- **协调器取消安全（v16）**：archive 与 DELETE 各一 barrier 用例——**DB blocking closure 已开始后取消 HTTP handler：并发 admission 持续被拒（reservation/deleting 不随请求 drop 释放），最终只能观察到已删除/已 tombstone 或确定失败后的 live**，绝无中间态泄漏；raw destructive API 可见性收窄的编译期/守卫测试。
- **lease 生命周期（v13/v14）**：`run_index` 已注册、transcript `run_start` 未提交时并发 archive → 被 lease 挡住；**persistence 首提交失败两阶段断言（v14/v15）——①失败发生而 provider 仍被 barrier 阻塞：lease 在持，archive 409、DELETE 触发 abort-and-drain；②provider 真正 terminal/error/cancel 后：lease 才释放**；普通 Started 成功 terminal 释放、provider error/panic terminal 释放；`QueuedToActiveRun` 成功 → 本请求 lease 释放无泄漏（既有 active lease 接管输入）；future cancellation/drop → 释放；同线程并发 follow-up → 独立 lease、排队语义不变；**DELETE abort-and-drain：deleting 态挡新 admission → 取消 lease → 等 task 终止 + lease 归零 → 删除（等待确认非仅 task.abort()）**；archive 对活 lease 409；创建回滚删除（无 lease）原语层直接放行。
- **validation 闭环（v12）**：invalid→valid→invalid **无重启**转换（update/upsert 路径重算）；`run_now` 与 scheduler 两条执行路径 fail-closed 重验；`validation_error` 不序列化落盘；stale validation 不得绕过。
- **统一 admission 与旁路封死（v11）**：**全停用 + 不存在的显式 `threadId` → 404 且 bridge/affinity 零调用**；**barrier 测试：缓存 bot→T 后 archive/delete T 再恢复 chat start → typed stale/not-found 或经 Fresh gate（全停用时 NoEnabledAgent），bridge/affinity 零调用；HTTP 与 WS 共用 prepare 同一合同双验**；真实 legacy 无 stamp thread → 兼容 fallback 照常续跑；thread-less cron 组合（config/磁盘两来源 × AgentTurn/SystemEvent × disabled/全停用 × **存在 enabled agent（拒因=缺 canonical target 非 gate 假绿）**）→ load 标 invalid + 派发 typed 拒绝 + automation list 可见 invalid 行可修复、`/api/cron/jobs` 输出校验状态、bridge 零调用；`ProviderTool` 通道拒绝 thread::/cron:: id；`garyx tool image` / `/api/tools/image` 在全停用下照常工作。
- **创建 gate**：Fresh/Fork/Recovered；bot 入站不 fallback/不虚构 key/明确文案；`/api/chat/start`、`/newthread`、task 通知首建；**全停用下 Add Bot 保存成功 + 首次入站返回 NoEnabledAgent 的闭环**；TaskService 五来源 + 必填构造；assign 区分；task 工作区等价。
- **automation**：generated/target 拆分；**legacy target job 显式错值（目标 Codex、job Claude）→ 派生重算后四端展示与运行一致**；**typed wire contract：`agent_resolution` 三态 + nullable `effective_agent_id` 在 desktop mapper/iOS models（无 claude 回填）/iOS cache（升版本）/CLI 全链解码**；**target→generated 转换 enabled/disabled/全停用三态**（disabled current 被 gate 拒）+ **目标删除、启动后无绑定线程经 task assign 获得绑定再转换（current 取实时绑定非启动缓存）、目标缺失/无绑定转换须显式重选**；归一化幂等（config 覆盖源二次启动）；Log/InternalDispatch 不受影响；停用后只改名不拒绝不改绑（三层）。
- **反向（不拦）**：disabled 既有线程续聊、thread send 返工、target-existing 创建与运行、generated 该次失败错误可见、全停用下已有线程 composer 照常。
- **desktop**：agent-options 过滤、默认预选、AddBotDialog、disabled 行动作、route 哨兵、client/bot console 映射、side-chat legacy fork、空态（限新草稿）；`test:unit` + `build:ui` + `build:web`。
- **iOS**：Core SwiftPM（过滤、decodeIfPresent、快照丢弃、两态、bot picker、nullable 选中态、effective 解码）+ `xcodebuild`。
- **CLI**：list、enable/disable/default、channels picker、task create disabled 报错。

## 5. 修订记录

- v16（2026-07-16）：回应十五轮 2 条——①archive/DELETE mutation 所有权归 store/coordinator 而非 HTTP future：请求取消/drop 不释放 reservation/deleting，blocking DB 操作（spawn_blocking 不可 abort）确定完成/失败前不恢复 live，终态二值可观察；补两个取消 barrier 测试；②删除"boot-only 豁免"错误事实（legacy_boot_import.rs:823 是 #[cfg(test)] 测试桩），raw 破坏性 API（garyx_db::archive_thread_record:650 / delete_thread_record_with_projections:2036）收窄可见性、HTTP route 改走协调原语、测试保留则加生产 call-site 精确守卫。
- v15（2026-07-16）：回应十四轮 4 条——①SPI 守卫改 CI 精确 call-site allow-list（`get_provider` 公开 live handle 使 deny-list 失效，`dashboard.rs:89`/`mcp/helpers.rs:69` 已持有）；②archive/DELETE 收敛为唯一行为矩阵（archive 先赢 reservation / lease 在持 archive 409 / DELETE 恒 abort-and-drain 非 409 / persistence 失败期分行为 / DELETE 中途失败 `deleting→live` 转移），清除全部"或"措辞；③archive 在任何 endpoint detach 前原子「确认无 lease + 设 reservation」，失败路径释放、lease 冲突零副作用，`legacy_boot_import.rs:823` 列 boot-only 豁免；④DELETE 状态机失败恢复：token 强制失效（bridge 入口前可见）、drain 不持锁防死锁、delete 失败清 deleting 不留永久拒绝态。§2 补 get_provider/archive 顺序/boot import 事实，§4 补对应测试。
- v14（2026-07-16）：回应十三轮 3 条——①`run_graph::execute_agent_run`/`RunGraphState` 收窄 crate 内部，`ProviderRuntime::run_streaming` 定性 provider SPI 例外，不变量边界收窄为「生产 crate 可达的 run 启动入口」+ deny-list 接口测试 + 生产调用树佐证；②删除策略拍板不留"或"：archive=活 lease 409、DELETE=abort-and-drain（deleting 态挡新 admission→取消 lease→等 task 终止与 lease 归零→删），lease 约束落 `ThreadStore` archive/delete 原语层单一咽喉自动覆盖 router 删除原语与创建回滚；③persistence 首提交失败改两阶段断言（失败≠run 结束：provider 阻塞期 lease 在持、terminal 后才释放），补 Started/error/panic terminal 释放用例。§2 补 run_graph 公开面、DELETE abort-then-delete 现行为、router 删除原语、persistence 失败语义事实。
- v13（2026-07-16）：回应十二轮 2 条——①`run_inline_streaming` 收窄为测试/bridge 内部可见，编译期封闭性测试覆盖所有能启动 provider run 的公开方法；②run intent 升级为**完整 lease 生命周期合同**：store-visible/per-request/cancel-safe；Started 原子转 active lease（archive 判断以 lease 域为准，覆盖 transcript `run_start` 异步落盘窗口）、QueuedToActiveRun 确认接管后才释放、error/取消/drop 不泄漏、同线程并发独立 lease 保排队语义；archive/独立 DELETE/hard delete（`routes.rs:3198-3259`、`:1136-1173`）全量纳入同一 lease 域。§2 补 run 生命周期细部事实，§4 补 lease 五场景测试。
- v12（2026-07-16）：回应十一轮 3 条——①admission 改 **sealed `AdmittedRun`**：字段私有生产外不可构造、裸 `start_agent_run`/`dispatch` 收窄可见性、包装层改签名；`ThreadBound` 只能由 store-backed admission 返回（不收调用方 record）；`ProviderTool` sealed provider 字段 + 构造时剥除绑定身份 metadata（封 `tool::*+agent_id` 混入）；②**原子 admission = 校验 record + 注册 run intent 同一线性化域**，archive/delete 消费同域：delete 先赢→stale、admission 先赢→archive 冲突，intent 接管/失败双路径释放；③validation 闭环：唯一纯 validator 在 load merge + 全部在线 mutation 后重算、`run_now`/scheduler 派发前 fail-closed 重验、`#[serde(skip)]` 不落盘。§2 补十一轮封闭性/TOCTOU/mutation 路径事实，§4 补编译期封闭、ProviderTool 反例、TOCTOU 双序 barrier、无重启 validation 转换测试。
- v11（2026-07-16）：回应十轮 3 条——①以**统一 typed run admission**取代入口枚举：`ThreadBound`（type-level 持真实 canonical record 构造，existing 来源统一重验；bot endpoint 快照 stale → fresh re-resolve 或 Fresh gate，封死十轮反例；`cron::` scheduled-reply 特判一并不可达）/`ProviderTool`（独立通道仅 `tool::*`）两态；②cron invalid 的 validation wire 合同：`CronJob.validation_error` 派生字段 + `AutomationSummary.validation_state/validation_error`，放宽 `is_automation_job` 使 invalid AgentTurn 可见可修、summary 转换产出 invalid 行不整体失败、SystemEvent 留 `/api/cron/jobs` 加校验输出、三端展示 + "存在 enabled agent"反假绿测试；③修正 §2 重复引用（`lifecycle.rs:190` 收敛为单条）。
- v10（2026-07-16）：回应九轮——新增不变量「agent run 只能携带真实存在的 canonical thread 绑定启动」，封死三条直达 bridge 旁路：①`/api/chat/start` 显式 threadId 必须真实存在否则 404（隐式创建唯一通道 = Fresh creator/gate），legacy 真实无 stamp 线程守恒；②`cron::` 伪 thread 直发分支退役——thread-less 的 AgentTurn/SystemEvent 组合 load 标 invalid + 派发 typed 拒绝（不选补建 canonical thread 路线，避免静默改 legacy 运行形态）；③provider 工具（tool::image::*）定性基础设施例外（无 agent 绑定、不选 agent），可被用户否决；④修正引用 lifecycle.rs:168→:190。§4 补三条旁路测试。
- v9（2026-07-16）：回应八轮 3 条，**以结构性收缩取代继续加协议**——承认 v5-v8 的 bridge 发布协议是过度设计：bridge 全部路径都是既有绑定续跑（历轮入口穷尽反复确认），不该是 enabled/default 决策点。①撤除 reconcile 通道/coherent unit/last-published 读面/双版本向量整套机制（八轮 #1 的 dirty-vs-mutation 矛盾与 config 事务、#2 的 cold-start 首发布、#3 的 provider identity 三个问题的前提随之消失）；②plugin/channel None 的默认解析前移到 creator 创建时 stamp canonical agent_id，bridge 照现状读 thread 绑定，legacy 无 stamp 线程 claude fallback 守恒；③决策点（gate/effective/list）同步读 store（与 mutation 同锁序线性化），cold-start 无新依赖；④bridge 传播保持现状 replace_agent_profiles 仅加单调 revision 丢旧；config/topology 热更与 provider 生命周期本特性零触碰。store clone→persist→swap 合同保留（六轮已同意）。
- v8（2026-07-16）：回应七轮 2 条——①channel-owned latest-state：触发源只投 dirty 标记，通道出队时读双源最新已提交版本（coalescing）后构造 unit，generation 在选定最新双版本后分配，unit 记录 `(config_revision, agent_state_revision)`；②单一线性化点：agent mutation 管线重排为 staged 构造 → persist → swap → 原子 publish（前两步可失败且失败即全不变），所有决策读面（Fresh/Task gate、effective default、bridge 路由）统一到 last-published snapshot，消除 swap 与 publish 间可见窗口与失败分裂；另按核验把 task assign 措辞修正为"补绑定"（`tasks.rs:825-847`，不同绑定被 `:776-780` 拒）。
- v7（2026-07-16）：回应六轮 3 条——①reserved 键集精确化：仅 `agent_id`/`requested_provider_type`/`provider_env`（garyx-models 共享常量），model/reasoning/tier/system prompt 保持 fill-only 优先级契约；HTTP/WS 与 CreateThreadBody 各自真实边界分别清剿；②bridge 版本域拆分：`agent_state_revision`（拒旧 agent snapshot）+ `reconcile_generation`（每次发布递增、排序整个 unit），全部刷新入口收敛单一 reconcile 通道，staged 构造 + 原子 swap（禁 pre-publish 原地写共享默认）；③automation typed wire contract：`agent_resolution` 三态 + nullable `effective_agent_id` 全链适配（禁哨兵串），target→generated 的 current 实时读目标线程、目标缺失/无绑定须显式重选。
- v6（2026-07-16）：回应五轮 4 条——①绑定身份 metadata 键（agent_id/requested_provider_type 等）定义为 server-owned reserved，外部边界与 provider_env 同咽喉剥除、服务端 canonical 强制覆盖（or_insert→insert），不提供 metadata one-off override；②automation 合同重构：范围限 AgentTurn、generated 仅缺失/空白归一、target-existing 的 agent 改为从目标线程实时派生（修历史显式错值）、target→generated 转换按新绑定强制 gate；③bridge 一致性：snapshot 带锁内单调 revision + 应用端丢弃旧版 + (snapshot, topology) 单一 reconcile 通道 coherent 发布；④store 变更合同 clone→persist→swap，失败不改内存不推送。
- v5：统一判据「enabled 只约束新绑定」；bridge 注入通道；删除 raw default 清空；automation 归一化常驻幂等；side-chat/事实修正；bot effective 客户端传播。
- v4：channel 三态传播补全；automation 显式存储（v5/v6 演进为按模式拆分+派生）；ResolvedAgentBinding 带工作区；badge/空态。
- v3：automation 客户端合同；channel 三态；TaskService fail-closed；NoEnabledAgent；管理面动作。
- v2：创建意图豁免 recover；typed AgentDisabled；TaskService 咽喉；versioned envelope；current 档；CLI channels picker；iOS 两态；PUT 三态。
