# Agent 创建/编辑身份规则与头像生成交互统一设计（终稿）

> 状态：设计终稿，不含实现
>
> 任务：#TASK-2233
>
> 范围：iOS Agent 创建/编辑身份规则；Mac + iOS Agent 头像生成与替换流程
>
> 裁决：以 Codex 版为主干，吸收 B 版指定发现；末尾附采纳/拒绝台账

## 1. 一页方案概要

本设计统一同一张 Agent 创建/编辑表面上的两件事，但不改变 Agent 写入契约。

### 1.1 iOS 身份字段对齐

- `Name` 是必填字段；iOS 标签改为 `Name`，移除当前错误的 `Optional` 提示。
- iOS 创建时，用户只填写 Name；Agent ID 由 `GaryxMobileCore` 按 Mac 当前 `deriveId(name)` 的精确语义实时派生，并以只读值展示。
- iOS 不保留手填 ID 的 escape hatch，也不引入 `agentIdTouched` / `isProgrammaticIdChange` 等 binding 守卫。
- iOS 编辑时 ID 始终是权威 profile 的原 ID；改 Name 不改 ID。
- 不自动加 `-2`、不静默覆盖，也不把 `POST` 改成 upsert。创建冲突继续由 Gateway 的 strict create `409` 最终裁决；iOS 把冲突定位回 Name。
- 编辑先读取权威 profile，以其 `updated_at` 做 `expected_updated_at`；并发 `409` 保留草稿，要求用户显式 Reload，不自动用旧草稿覆盖新数据。

### 1.2 头像生成统一交互

- Mac 保留 Agent 表单顶部 Avatar 区与 compact dialog 的信息架构；iOS 用原生 `NavigationStack + Form + .medium/.large sheet detents` 适配。
- 选择样式、生成中、候选预览、失败与重试都留在同一张聚焦子表面；开始生成后不自动关闭。
- 生成中在头像预览上原位显示“半透明遮罩 + spinner + 状态文字”，可加低对比 shimmer/sweep；不伪造百分比。
- 生成成功先产生候选头像。已有头像时显示 `Current` / `New` 对比；只有点击 `Use avatar` 才把候选写入 Agent 表单草稿，最终仍由整个表单的 Create/Save 持久化。
- 失败保留已选样式、自定义 prompt、当前头像和可用候选，原位显示 Retry；不能只靠短暂 toast。
- 取消是真取消：iOS `Task`/URLSession cancel；Desktop 通过 scoped cancel IPC 中止 Electron main 的 `AbortController`；Gateway 在 `/api/tools/image` 请求 future 被取消/连接断开时 abort 对应 bridge run。
- Desktop 同时使用“代际 counter/token + request ID 归属检查”抵御迟到结果。AbortController 负责停止工作，token/request ID 负责即使 abort 竞态失败也不让旧结果改 UI。

主方案不新增公开 Gateway 路由，也不改变 `/api/tools/image` 的请求/响应结构；只补齐请求生命周期取消到 provider run 的传播。

## 2. 目标、边界与术语

### 2.1 目标

- 两端对 Name 的标签、必填性、保存校验和头像生成前置条件形成一致心智。
- iOS 的 name → ID 规则与 Mac 当前实现保持原文级、可测试的一致。
- 用户不需要理解或手填 ID，也不会在 iOS 编辑一个最终被 Gateway 忽略的 ID。
- 头像生成的触发、等待、成功、失败、重试、取消和替换都有稳定、可恢复的状态。
- 替换已有头像前可比较并明确确认。
- 不破坏 strict create 与乐观并发更新契约。

### 2.2 不在本设计范围

- 不实现代码，不发布，不合并 main。
- 不改变 Agent provider、model、environment、workspace、system prompt 的产品含义。
- 不让头像生成绕过 Agent 表单单独持久化。
- 不设计 determinate 百分比；当前 Gateway/provider 没有可信进度数据。
- 不改变 `agent_id` 的长期引用与路由语义，也不设计 Agent rename/migration。
- 不新增异步 image job、轮询或 SSE 进度 API。
- 不在本任务中删除 Mac create 的手动 ID override；该一致性问题保留为开放项。

### 2.3 术语

- **Name**：UI 字段；wire 字段仍为 `display_name`。
- **ID**：`agent_id`；创建后不可变，用于 URL、选择和运行时引用。
- **当前头像（Current）**：进入头像生成子流程时，Agent 表单草稿已经采用的头像或占位符。
- **候选头像（New）**：生成成功但尚未点击 `Use avatar` 的图片。
- **采用**：点击 `Use avatar`，只更新本地 Agent 表单草稿。
- **持久化**：点击整个 Agent 表单的 Create/Save，并经 Gateway 写入 profile。
- **代际 token**：renderer 内单调递增的 generation counter；用于判定某个异步回调是否仍属于当前头像操作。
- **request ID**：一次具体头像操作的稳定标识；用于 UI 归属检查和 scoped IPC cancel。

## 3. 取证范围与真相层级

| 主题 | 事实来源 |
| --- | --- |
| Mac 表单、name → ID | `desktop/garyx-desktop/src/renderer/src/app-shell/components/AgentsHubPanel.tsx`、`AgentFormDialog.tsx`、`agents-hub-helpers.ts` |
| Mac 头像 UI 与请求 | `AgentAvatarEditor.tsx`、`AgentsHubPanel.tsx`、`styles/agents-hub.css`、`desktop/garyx-desktop/src/main/agent-avatar.ts` |
| iOS 表单与头像 UI | `mobile/garyx-mobile/App/GaryxMobile/GaryxMobileAgentsViews.swift`、`GaryxMobileAgentPickerComponents.swift` |
| iOS 业务与网络 | `GaryxMobileModel+AgentsWorkspaces.swift`、`Sources/GaryxMobileCore/GaryxGatewayAgentModels.swift`、`GaryxGatewayClient.swift` |
| Agent 写契约 | `docs/agents/repository-contracts.md`、`garyx-gateway/src/api.rs`、`custom_agents.rs`、`optimistic_write.rs` |
| 图片生成 | `garyx-gateway/src/tool_image.rs` |

真相层级如下：

1. Mac 是信息架构、字段含义、标签和 style catalog 的产品真相源。
2. Gateway 是持久化、唯一性和写并发的契约真相源。
3. iOS 复制业务语义，但使用原生 iOS 布局与交互，不复制桌面网格或自定义玻璃内容面板。
4. iOS 的 name → ID、validation、collision、create/edit projection 与头像流程状态下沉 `GaryxMobileCore`，可用 SwiftPM 无 UI 测试。

平台设计依据是 Apple HIG 的 [Progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators)、[Loading](https://developer.apple.com/design/human-interface-guidelines/loading)、[Motion](https://developer.apple.com/design/human-interface-guidelines/motion) 与 [Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility)：未知时长使用持续、原位的 indeterminate 反馈；失败给出恢复动作；动效不能是唯一信息，并响应 Reduce Motion/Transparency。

## 4. 两端现状：Agent 创建/编辑

### 4.1 Mac：name → ID 的现行规则（原文级）

Mac 唯一转换函数位于 `agents-hub-helpers.ts:103-110`：

```ts
export function deriveId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}
```

创建表单在用户尚未手动碰过 ID 时，随 Name 实时同步（`AgentsHubPanel.tsx:201-207`）：

```ts
useEffect(() => {
  if (agentDialogMode !== 'create' || agentIdTouched) {
    return;
  }
  const nextId = deriveId(agentDraft.displayName);
  setAgentDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
}, [agentDialogMode, agentDraft.displayName, agentIdTouched]);
```

现行前端校验位于 `AgentFormDialog.tsx:192-199`：

```ts
const agentValidationError =
  !agentDraft.displayName.trim()
    ? t('Name is required.')
    : !agentDraft.agentId.trim()
      ? t('Agent ID is required.')
      : envRowsHaveInvalidKey(agentDraft.env)
        ? t('Environment variable names must match [A-Za-z_][A-Za-z0-9_]*.')
        : null;
```

Gateway 也要求 `agent_id` 和 `display_name` trim 后非空。Mac 的变换顺序精确定义为：

1. JavaScript `trim()` 去首尾 whitespace。
2. JavaScript `toLowerCase()`。
3. 每一段连续的非 ASCII `a...z` / `0...9` 字符替换为一个 `-`。
4. 删除首尾全部 `-`。
5. 把连续 `-` 再压成一个 `-`；即使第三步后通常冗余，该步骤仍是现行契约的一部分。

规则没有音译、Unicode 字母保留、最大长度、保留字、provider 前缀或 hash fallback。

| Name 输入 | Mac 当前输出 |
| --- | --- |
| `  Test Agent  ` | `test-agent` |
| `Agent__42` | `agent-42` |
| `Foo---Bar` | `foo-bar` |
| `Café Déjà` | `caf-d-j` |
| `你好 Agent` | `agent` |
| `研发助手` | 空字符串 |
| `---` | 空字符串 |

#### 4.1.1 去重与冲突的准确含义

“去重”必须分成两层：

- **字符归一化**：连续的非法字符段变成一个 `-`，首尾 `-` 被删除。
- **实体唯一性**：Mac 没有查列表后添加 `-2`，没有 ID 预占，也没有创建前的权威 uniqueness API。两个 Name 派生为同一 ID 时，原 ID 原样 POST。

最终唯一性由 Gateway strict create 裁决。`optimistic_write.rs:68-73` 的现行逻辑是：

```rs
WriteExpectation::Create => match stored_updated_at {
    Some(_) => Err(StoreWriteError::Conflict {
        message: format!("{what} already exists"),
        current_updated_at: stored_updated_at.map(ToOwned::to_owned),
    }),
    None => Ok(()),
},
```

因此已存在 ID 返回 `409`；不会 silent overwrite，也不会自动 suffix。

#### 4.1.2 Mac create/edit 语义

- create：Name 与 ID 都要求 trim 后非空；ID 初始由 Name 生成。
- create 当前仍允许手动编辑 ID。第一次编辑会设置 `agentIdTouched = true`，后续 Name 不再驱动 ID。
- edit：ID input disabled；Name 可改，ID 不随改名变化。
- update：Gateway 以 URL path 的 ID 为目标；payload 中的 `agent_id` 不构成 rename。
- Mac 使用所选 profile 的 `updatedAt` 作为 `expected_updated_at`；token 过期返回并发 `409`。

“Mac create 允许 override”是源码事实，不是 iOS 目标；老板的明确要求是 iOS 用户不再手填 ID。是否以后连 Mac 一起移除 override，见第 12.6 节。

### 4.2 iOS：当前字段与校验

iOS 当前 Identity section 先放可编辑 `Agent ID`，再放 `Display name`。后者明确写着错误的 optional 提示：

```swift
GaryxFormTextFieldRow(
    title: "Display name",
    text: $displayName,
    placeholder: "Optional"
)
```

但 `canCreate`、`canSaveAgent` 以及 model 的 create/update guard 都同时要求 ID、display name、provider 非空。所以 Name 实际必填，UI 却宣称可选；Save 变灰时也没有字段级原因。

iOS 目前没有 Mac `deriveId` 的等价实现：create 的 ID 从空字符串开始，由用户手填；edit 也把 ID 当可编辑 TextField。

edit 的假交互更严重：

- view 把用户编辑后的 ID 传入 `updateAgent`。
- client 的 PUT URL 仍使用原 `agent.id`。
- payload 带入编辑后的 `agent_id`，但 Gateway handler 用 URL ID 构造 store request。
- 保存可以成功，返回 ID 仍为原值；用户看到的是一次静默 no-op。

### 4.3 当前字段对照

| 字段 | Mac 当前 | iOS 当前 | Gateway/目标约束 |
| --- | --- | --- | --- |
| Name | 标签 `Name`，必填 | `Display name` 写着 Optional，但实际必填 | `display_name` trim 后必须非空 |
| Agent ID | create 自动跟随但可 override；edit disabled | create/edit 都可手填 | create 必填且唯一；update 由 URL ID 定位 |
| Provider | 必选；新建默认 Claude | 必选；新建当前默认 Codex | 必填 enum；默认差异不在本任务顺改 |
| Model | provider 支持时可选 | 可选 | 空表示 provider default |
| Thinking level | capability 支持时可选 | 可选 | update 需按现有显式/继承语义 |
| Service tier | Mac 可显示/编辑 | iOS 表单不显示 | iOS update 必须保留权威 profile 值 |
| Environment | key 匹配 shell identifier | 已有同类校验 | `[A-Za-z_][A-Za-z0-9_]*` |
| Default workspace | 可选 | 可选 | 空表示未设置 |
| Avatar | Upload/Generate/Clear | Upload/Generate，无 Remove | data URL；update 中空字符串与省略语义不同 |
| System Prompt | 可选 | 可选 | 空值允许 |

### 4.4 iOS 当前更新并发路径

iOS 已能发送 `expected_updated_at`，且恢复自缓存或 token 缺失时会重新 list；但普通 edit flow 仍可能直接以展示 summary 建草稿。仓库契约要求每个 edit flow 基于 fresh profile。目标应在进入编辑时调用 `GET /api/custom-agents/{id}`，用返回 profile 一次性建立草稿与基线 token，不能把列表投影当写入真相。

## 5. 两端现状：头像生成完整流程

### 5.1 共同链路与 Gateway

两端当前都调用同步等待型 `POST /api/tools/image`：

1. 客户端依据 Name/ID、style prompt 与共同 composition 规则构造 prompt。
2. Gateway 在私有 tool workspace 启动强制 Codex App Server 的 run。
3. Gateway 等待 `imageGeneration` tool result；默认 timeout 600 秒，最大 900 秒。
4. timeout 时 `tool_image.rs:178-183` 主动调用 `abort_run`；成功返回 base64、media type、runtime thread ID 与 run ID。
5. provider 无图、损坏图、bridge/IO 等错误映射为 502，timeout 为 504。

当前 API 没有 progress event、job status 或 cancel route。更关键的是：`run_image_tool` 只有 timeout 分支 abort；HTTP request future 因客户端取消/断连而被 drop 时，没有持有一个会 abort 已启动 run 的 request-lifetime guard。

Mac 与 iOS 的 8 个固定 style prompt 及最终 avatar prompt 当前语义一致，但分别维护在 TypeScript 与 Swift 中，没有跨端 parity fixture。

### 5.2 Mac 当前流程

1. **入口**：create/edit dialog 顶部是 48×48 preview，旁边有 `Upload avatar`、`Generate avatar`；已有头像时有 `Clear`。
2. **触发**：Generate 打开 `Avatar style` compact dialog；包含 8 个固定样式和 Custom style。
3. **前置**：Name 或 ID 任一非空即可生成；Custom style 必须非空。
4. **请求**：renderer 调 IPC；Electron main 构造 prompt，POST `/api/tools/image`，再把图片归一化为 256×256、优先 PNG、过大时 JPEG。
5. **等待**：主要反馈是两个 Generate 按钮变为 `Generating...`。头像 preview 不动，没有 overlay、状态说明或阶段感知。
6. **成功**：`AgentsHubPanel.tsx:339-344` 直接写 `agentDraft.avatarDataUrl`、关闭 style dialog、显示 `Avatar generated` toast。
7. **失败**：catch 丢掉具体错误，只显示通用 `Failed to generate avatar` toast；dialog 没有失败态或 Retry。
8. **关闭/竞态**：style dialog 的 footer Cancel 在生成时 disabled，但 Dialog 的关闭路径没有与 request cancel 建立统一契约。renderer 只有 `avatarGenerating` boolean，没有 counter、request ID 或迟到结果守卫；main 只有 timeout signal，没有用户 cancel controller registry。
9. **替换**：成功立即替换草稿，不提供 Current/New 比较；最终 Save 才持久化。

### 5.3 iOS 当前流程

1. **入口**：Form 顶部 Avatar section 显示 76pt preview，提供 PhotosPicker Upload 与 Generate。
2. **触发**：ID 或 Name 任一非空时可生成；点击后打开 `.fraction(0.93)` / `.large` 的自定义 sheet，内容由自定义 glass panels、渐变背景和底部 material slab 组成。
3. **时序缝隙**：style sheet 的 Generate action 是：

   ```swift
   Button {
       let prompt = activeStylePrompt
       dismiss()
       onGenerate(prompt)
   } label: {
       // ...
   }
   ```

   `dismiss()` 在 `onGenerate` 之前；sheet 退场已经开始，父层才创建 Task、调用 `editorState.begin`。因此传入 sheet 的 `isGenerating` spinner/disabled UI 通常没有机会成为可见的持续状态，立即失败也只能落到已返回的表单/全局反馈。这是明确的 P9 时序缝隙；`onGenerate` 本身是同步回调而非 throwing closure，问题核心是状态启动晚于 dismiss，而不是“异常穿过 dismiss”。

4. **等待**：用户回到表单，只在 Generate capsule 中看到小 `ProgressView` 和 `Generating`；没有 preview overlay、说明或显式 Cancel generation。
5. **已有安全性**：`GaryxMobileAvatarEditorState` 使用 request ID + fingerprint；fingerprint 包含 ID、Name、provider 和当前头像。Task 取消或 fingerprint 变化后，迟到结果不会应用。
6. **成功**：结果归一化、predecode 后，`GaryxMobileAgentsViews.swift:730` 直接执行 `avatarDataUrl = generated`；无 candidate、成功文案、haptic 或过渡动画。
7. **失败的两层事实**：
   - model 的 `generateAvatar` 在输入非法、空结果、normalization 或网络/provider 错误时会写 `model.lastError` 并返回 `nil`；root status host 观察 `lastError`，约 3.2 秒 toast 仍可能显示。因此不能笼统写成“用户一定完全看不到错误”。
   - 但 `GaryxAvatarEditorSection.generateAvatar` 的现行代码是：

     ```swift
     guard let generated = await onGenerate(stylePrompt) else { return }
     ```

     `nil` 分支静默 return，不调用 section 已有的 `onError`，也不进入局部 failure/Retry 状态。相反，upload 的读图失败、normalization 失败和普通 catch 都会在 request 仍有效时调用 `onError(...)`。这是一个经源码核准的明确不一致：**全局 toast 可能出现，但 section-local 生成失败路径被绕过。**
8. **离开/取消**：section `onDisappear` 会 `Task.cancel()` 并 reset request state，但 UI 没有可见取消入口；当前 Gateway 也不保证 URLSession 取消最终终止 provider run。
9. **替换**：成功立即替换；当前 iOS 没有 Clear/Remove。最终 Save 才持久化。

### 5.4 当前等待/失败/替换对照

| 阶段 | Mac 当前 | iOS 当前 |
| --- | --- | --- |
| 样式表面 | compact Dialog | 0.93 fraction 自定义 glass sheet |
| 启动 | dialog 保持 | 先 dismiss，再创建 generation Task |
| preview loading | 无 | 无 |
| 按钮 loading | `Generating...` | 返回表单后的 ProgressView；sheet 内通常不可见 |
| 成功 | 直接写 draft、关 dialog、toast | 直接写 binding，无成功状态 |
| 失败 | 通用 toast，丢具体原因 | model 写全局 toast；section nil 静默 return，无局部失败态 |
| 取消 | UI 关闭不等于请求 abort | 仅生命周期隐式 Task cancel |
| 迟到结果 | 无 guard | request ID/fingerprint guard |
| 替换确认 | 无 | 无 |

## 6. 头像生成“不丝滑”痛点清单

下表保留 B 版 P1–P9 的编号并按源码校正，再补充主稿覆盖的系统性问题。

| # | 严重度 | 端 | 痛点与证据 |
| --- | --- | --- | --- |
| P1 | 高 | 两端 | **preview 无 loading 反馈。** 生成期间大头像保持首字母/旧图不动，只靠按钮文字或小 spinner；用户缺少稳定的注视点。 |
| P2 | 高 | 两端 | **失败反馈弱且失去上下文。** Mac 抹平具体错误；iOS style sheet 已关，只剩短暂全局 toast，style/custom prompt 与 Retry 不在错误旁。 |
| P3 | 中 | 两端 | **换图没有过渡动画。** Mac `AgentAvatarEditor.tsx:49-50` 直接渲染 `<img>`，`agents-hub.css:200-207` 只定义尺寸/裁切、无 transition/animation；iOS `GaryxAgentAvatarView:123-159` 没有 `.transition`/`.contentTransition`，赋值点 `GaryxMobileAgentsViews.swift:730` 也没有 `withAnimation`。新图会“啪”地切换。 |
| P4 | 中 | Mac | **成功自动关闭 style dialog。** 不满意时必须重新打开、重新进入上下文，无法在同一表面继续迭代。 |
| P5 | 高 | Mac | **只有 busy boolean，没有结果归属保护。** close/reopen、form close、未来允许 retry 后，旧请求可能覆盖新状态；旧 `finally` 也可能清掉新请求的 busy。 |
| P6 | 高 | iOS | **generation nil 绕过 `onError`。** section 的 `guard ... else { return }` 与 upload 多处 `onError(...)` 不一致；虽然 model 可能触发全局 toast，但局部 failure/Retry 无法建立。 |
| P7 | 高 | 两端 | **选择 → 等待 → 结果是断裂流程。** 没有在同一表面内连续选样式、生成、比较、再生成的闭环。 |
| P8 | 中 | 两端 | **未知长等待缺少进度感知。** 请求可能持续几十秒甚至数分钟，却没有持续的 preview activity 或“可能需要一点时间”的说明；也不能伪造百分比补救。 |
| P9 | 中 | iOS | **dismiss 先于 generation start。** `dismiss()` → `onGenerate(prompt)` 让 sheet loading UI 先退出，父级 begin/task 后发生；失败/取消/生命周期状态跨表面，形成可观察的时序缝隙。 |
| P10 | 高 | 两端 | **生成成功即覆盖 Current。** 生成式结果不可预期，但没有 Current/New 比较与明确采用动作。 |
| P11 | 高 | 两端/Gateway | **取消语义不真实。** Mac 关闭 dialog 不一定停 HTTP；iOS cancel 只保证本地 Task；Gateway 只在 timeout abort，没有 request-drop cleanup。 |
| P12 | 中 | iOS | **fingerprint 可静默丢结果。** 用户在等待时改 Name/provider/avatar，结果被正确拒绝但没有解释；安全机制变成隐形失败体验。 |
| P13 | 中 | iOS | **样式 sheet 偏离原生管理面。** 0.93 自定义 detent、整页渐变、glass cards 与自制底栏增加视觉重量，也不符合仓库 grouped Form 规范。 |
| P14 | 中 | 两端 | **style/prompt 双份维护缺少护栏。** 当前内容一致，但无 parity fixture，未来容易漂移。 |

应保留的现状优点：头像仍只进入表单草稿；两端均做小尺寸 normalization；iOS 已有 request ID、防迟到结果和 predecode；8 fixed styles + Custom style 的 IA 已经一致。

## 7. iOS 身份字段对齐方案

### 7.1 目标表单结构

iOS create/edit 继续使用原生 grouped management surface，Identity section 顺序调整为：

1. `Name`：可编辑、必填，不再叫 `Display name`，不再显示 `Optional`。
2. `Agent ID`：只读 `LabeledContent`/read-only row，等宽 secondary value；不是 disabled TextField。
3. create footer：`Agent ID is generated from Name.`
4. edit footer：`Agent ID can’t be changed after creation.`

create 中 ID 随 Name 实时派生；edit 中始终显示权威 profile ID，Name 变化不触发 ID 变化。只读展示保留了可诊断性：纯 CJK/全标点 Name 生成空 ID 时，用户能理解保存为什么受阻。

### 7.2 GaryxMobileCore 所有权

新增纯值规则层，命名示意为 `GaryxCustomAgentDraftRules` / `GaryxCustomAgentEditDraft`。它至少拥有：

- mode：`.create` 或 `.edit(existingId, expectedUpdatedAt)`；
- raw Name、trimmed Name；
- create 的 derived ID / edit 的 immutable ID；
- provider、env 等 validation 输入；
- 与某个 derived ID 绑定的 server collision；
- 字段级 validation result 与 `canSubmit`；
- create/edit wire payload projection；
- 未展示字段（如 service tier）的保留策略。

SwiftUI view 只负责 binding、焦点、显示和启动副作用。**不设计** `agentIdTouched`、`isProgrammaticIdChange` 或可编辑 ID binding；因为 ID 是纯派生只读值，这整套 guard 复杂度没有存在理由。

### 7.3 name → ID 的 iOS 等价语义

Core 按固定顺序实现：

1. 匹配 JavaScript `trim()` 的首尾 whitespace 语义；
2. lowercase；
3. ASCII 正则语义 `[^a-z0-9]+` → `-`；
4. trim 首尾 `-`；
5. collapse `-{2,}`。

不能使用通用 slug/transliteration 库，不能保留 Unicode 字母，不能加 hash fallback。Swift/Foundation 的 whitespace/lowercase 行为必须通过同一组 TypeScript golden fixture 验证，不能只凭“看起来等价”判断。

如果将来要支持纯 CJK Name，应先改变 Mac 真相源并设计跨端迁移；iOS 不单独变聪明。

### 7.4 逐字段与逐校验

| 字段 | create 目标 | edit 目标 | 校验/保存语义 |
| --- | --- | --- | --- |
| Name | 用户输入，必填 | 用户输入，必填 | trim 后空：`Name is required.`，聚焦 Name |
| Agent ID | Name 实时派生，只读 | 权威 profile 原 ID，只读 | create 派生为空：`Name must include at least one English letter or number.` |
| Avatar | Upload/Generate；候选确认后进草稿 | 同左；Current 到 Use 前不变 | optional；生成前复用 Name 校验 |
| Provider | 保持现有 picker/default | 权威 profile 初始化 | trim 后非空；不在本任务顺改默认 provider |
| Model | optional/provider default | 权威 profile 初始化 | 不发送 capability 不支持的本地陈旧值 |
| Thinking level | optional/provider default | 权威 profile 初始化 | 按现有 capability 与显式空语义 |
| Service tier | 不新增 UI | 不新增 UI | update 必须保留权威值，不能因未展示而清空 |
| Environment | optional | 权威 profile 初始化 | key 匹配 `[A-Za-z_][A-Za-z0-9_]*`，错误定位 section |
| Default workspace | optional | 权威 profile 初始化 | trim 后空表示未设置 |
| System Prompt | optional | 权威 profile 初始化 | 空值允许 |

提交状态应区分：

- `invalid`：Create/Save disabled，同时字段下方显示原因，不能只呈现灰按钮。
- `saving`：所有重复提交入口 disabled，toolbar 使用固定尺寸 spinner，避免文字变化导致跳动。
- `server error`：表单保持打开，草稿、已采用的新头像和未保存编辑不丢失。

### 7.5 create 冲突：strict 409

1. 以当前 Name 的 derived ID 发 `POST /api/custom-agents`。
2. `201`：创建成功。
3. `409`：解析 `GaryxGatewayError.httpStatus(409, body)`，映射为 Name 字段错误，例如：

   `An agent named “Test Agent” already uses the ID “test-agent”. Change the name and try again.`
4. collision 与触发它的 derived ID 绑定。Name 改动但 derived ID 仍相同时错误保留；只有 derived ID 改变才清除并允许重试。
5. 不自动发 `test-agent-2`，不改为 PUT，不自动重试非幂等 create。

本地 catalog 最多提供提示性预检，不能替代 POST；缓存和并发创建会让本地判断过期。主方案可以直接依赖权威 409，以保持语义简单。

### 7.6 edit 权威读取与并发冲突

1. 打开 edit sheet 后调用 `GET /api/custom-agents/{id}`。
2. 读取完成前可以显示 cached identity preview，但 Save disabled；权威 profile 到达后一次性建立草稿和 `expected_updated_at`。
3. 改 Name 只改 `display_name`；ID 固定。
4. PUT 的 URL ID 与 payload ID 都使用原 ID，并携带 fresh token。
5. `404`：提示 Agent 已删除，停止保存；不能用 PUT 复活。
6. `409`：保留本地草稿与头像，显示阻塞 inline banner：

   `This agent changed elsewhere. Reload the latest version before saving.`
7. `Reload latest` 显式 GET 并重置表单；不能只换 token 后自动重放整份旧 payload。

这保持仓库契约：POST 已存在为 409；PUT 缺 token 为 400、已删除为 404、token 移动为 409，并返回 `current_updated_at`；没有 unconditional HTTP write。

### 7.7 Core 测试矩阵

SwiftPM 无 UI 测试至少覆盖：

- 第 4.1 节全部 name → ID golden cases。
- whitespace-only、纯 CJK、emoji-only、重音字符、连续 separator、超长 Name。
- TypeScript 与 Swift fixture parity，特别验证 whitespace/lowercase 边界。
- create 改 Name 实时改 ID；edit 改 Name 不改 ID。
- iOS 没有手填 ID projection，也没有 touched/programmatic guard 状态。
- create collision 仅在 derived ID 改变后清除。
- create payload 不带 `expected_updated_at`；edit payload 必带 fresh token。
- provider required、env key invalid、未展示 service tier 保留。
- create 409、edit 404、edit 409 分别映射到不同 UI state。

## 8. 头像生成新交互设计

### 8.1 统一心智模型

```text
form idle
   │ Generate avatar / Generate new
   ▼
choosing style ── Cancel ───────────────► form idle（Current 不变）
   │ Generate
   ▼
generating ───── Cancel generation ─────► abort + form idle（Current 不变）
   │
   ├─ success ──► candidate ready ── Use avatar ──► form draft updated
   │                    │
   │                    └─ Generate again
   │
   └─ failure ──► failed ── Retry / Change style

form draft updated ── Create/Save ──────► Gateway profile persisted
```

头像生成是一个 focused transaction：进入时快照 Current、trimmed Name、ID、style/custom prompt。底层 Agent 表单由 modal/sheet 隔离，不能在等待中继续修改 prompt 归属字段；request ID 仍负责拒绝取消后或旧请求的迟到结果。

### 8.2 逐状态契约

| 状态 | 共同呈现 | 主操作 | 数据语义 |
| --- | --- | --- | --- |
| 触发 | Avatar 区显示 Current；无图为 `Generate avatar`，有图为 `Generate new` | 点击后先校验 Name | 校验失败聚焦 Name，不开子表面 |
| 选择样式 | 同一子表面显示 Current preview、8 fixed styles、Custom style | `Generate` | 捕获 immutable request snapshot |
| 生成中 | 子表面不关闭；preview 原位 loading overlay；style controls disabled | `Cancel generation` | 真 cancel；Current 与已有 candidate 不变 |
| 成功 | 停止 loading；已有图显示 `Current` / `New`，无图显示 placeholder / `New` | `Use avatar`；secondary `Generate again` | 结果先进入 candidate，不改 form draft |
| 失败 | preview 不清空；原位错误；保留 style/custom prompt 和可用 candidate | `Retry`；secondary `Change style` / `Cancel` | 不改 form draft |
| 重试 | 同 style/Name snapshot 创建全新 request ID 和 generation token | `Cancel generation` | 旧请求结果永不应用；不做后台自动 retry |
| 替换 | Use 后，form preview 从 Current crossfade 到 candidate | 整体 Agent `Create`/`Save` | Use 只改草稿；Save 成功才落库 |

Upload 保持并列入口。用户上传的是已明确选择的具体图片，可在 normalization + predecode 后直接进入表单草稿；若替换已有图，取消整个 Agent 表单仍可撤销。Mac 当前 `Clear` 统一改为较低强调的 `Remove avatar`；iOS 补同语义入口。edit 中 Remove 需要发送明确空字符串，不能与“avatar 未修改/省略字段”混淆。

### 8.3 候选与多次生成

- 子流程维护 `currentAvatar`、`candidateAvatar?`、`selectedStyle`、`customPrompt` 与 operation state。
- 第一次生成时 overlay 盖在 Current/placeholder preview 上。
- 有 candidate 后点击 Generate again：旧 candidate 可留在 `New` 槽位并加 loading overlay；新请求成功后才替换 candidate，失败时仍允许采用上一张 candidate。
- 关闭子流程但未点 Use：candidate 丢弃，Agent 草稿 Current 不变。
- 点 Use：candidate 进入 Agent 草稿，子流程关闭；之后 Agent Save 失败时该草稿仍保留。
- 生成成功不显示“已经保存”的措辞；`Avatar ready` 与 `New avatar selected` 分开表达。

### 8.4 Mac 具体呈现

Mac 保留 Agent dialog 顶部 Avatar editor 作为 IA 真相源：

- 表单 Avatar row 的 preview geometry 在 idle/loading/selected 间固定；action 顺序为 `Upload`、`Generate avatar`/`Generate new`、有头像时 `Remove avatar`。
- 点击 Generate 打开现有 `AvatarStyleDialog` 的演进版，不新增第三层表面。
- dialog 顶部增加 preview；candidate ready 后切换为 `Current` / `New` 比较。固定 style 保持紧凑 2-column grid，Custom style 保持同层 textarea。
- generating 时 style controls disabled；Generate footer 保持宽度并显示 spinner + `Generating…`；`Cancel generation` 始终可用。
- success footer：secondary `Generate again` + primary `Use avatar`。
- failure：错误放 preview 下方并 `aria-live="polite"`，primary `Retry`，secondary `Change style` / `Cancel`。
- X、Escape、footer Cancel 与 Agent form close 在 generating 时走同一 cancel transition；关闭后焦点回 Generate trigger。
- success/failure 的结果归属必须经过第 8.9 节的 counter + request ID 检查。

### 8.5 iOS 原生适配

iOS 复制同一信息顺序和状态，不复制桌面 grid/dialog：

- Avatar 继续是 grouped Form section；preview 居中，Upload/Generate/Remove 使用 44pt 以上原生 controls。
- Generate 打开系统 sheet，内部为 `NavigationStack` + grouped `Form`/`List`。
- navigation title：`Avatar style`。
- selection rows 使用普通 row + `GaryxSelectionCheckmark`；Custom style 使用带明确 label 的 vertical TextEditor row。
- toolbar leading：选择/候选/失败态为 `Cancel`；生成态为 `Cancel generation`。
- toolbar trailing：按状态为 `Generate` 或 `Use`；`Generate again` 放在 preview/status section 的 secondary action。
- 使用 `.presentationDetents([.medium, .large])`，键盘或内容需要时自然升到 large；不保留 `.fraction(0.93)`、自定义整页渐变、重复 glass cards 或自制 safe-area button slab。
- generating 时 `.interactiveDismissDisabled(true)`，避免下拉关闭产生“UI 消失但请求仍跑”的模糊语义；用户通过清晰可见的 Cancel generation 真取消。系统销毁/onDisappear 仍作为兜底 cancel。
- iPhone compact、iPad split view、Dynamic Type 下，Current/New 从横排自适应为纵排，不固定高度裁字。
- success 可触发一次 `.success` sensory feedback，failure 一次 `.error`；文本仍是主反馈。

### 8.6 Loading overlay 的具体形态与样式落点

共同视觉结构：

1. preview 容器保持原尺寸和圆角，不因状态插入/删除而跳布局。
2. Current 或 candidate 图像仍在底层可辨认。
3. 顶层覆盖与头像同 shape 的 28%–34% 中性色半透明遮罩。
4. 中心显示 indeterminate spinner；空间允许时 spinner 下方显示 `Generating avatar…`。
5. 可选一条低对比 shimmer/sweep，从左上到右下或水平扫过，约 1.8–2.2 秒循环；它只增加“仍在工作”的生命感，不承载进度含义，也不能替代 spinner/文字。
6. 超过约 8 秒，在同一 status 区增加 `This can take a little while.`；不新弹 toast，不伪造百分比。

**Mac 样式落点**：

- `AgentAvatarEditor.tsx` 为 preview 增加相对定位 wrapper，并根据 flow state 渲染 `.agents-hub-avatar-loading-overlay`、spinner、status text 与可选 sweep element。
- `styles/agents-hub.css` 定义 overlay 的 `position: absolute; inset: 0; border-radius: inherit;`、语义遮罩色、中心布局、sweep keyframes、candidate crossfade 和 reduced-motion media query。
- 优先复用现有色彩/背景 token；若缺 overlay token，使用能在 light/dark 下保持文字对比的中性 alpha，不用新造品牌色。

**iOS 样式落点**：

- 在头像生成专用 preview 容器中用 `ZStack`/`.overlay` 叠加同 shape 的半透明层、原生 `ProgressView` 与文本；不把 loading prop 泛化到所有列表头像。
- 可选 sweep 使用低对比 `LinearGradient` clip 到 Circle/RoundedRectangle；`accessibilityHidden(true)`，避免 VoiceOver 读装饰层。
- Reduce Transparency 开启时，把半透明/材质替换成足够不透明的 semantic fill + border；高对比下 spinner/文字仍满足辨识度。

### 8.7 动效与无障碍要求

| 转换 | 默认动效 | Reduce Motion |
| --- | --- | --- |
| idle → generating | overlay 150–180ms opacity；可选 sweep | overlay 即时/短 opacity；关闭 sweep |
| generating → candidate | predecode 后 180–220ms crossfade | 即时换图或 ≤100ms opacity |
| failure overlay | 150–180ms opacity；不依赖 shake | 即时显示 |
| Use → form preview | 约 180ms crossfade，无弹跳 spring | 即时换图或短 opacity |
| style selection | 120–150ms border/background | 即时状态切换 |

其他要求：

- 不对整个 dialog/sheet 做缩放或循环呼吸。
- 动效不作为唯一状态信号；始终有文字和 control state。
- ARIA/VoiceOver 只在状态边界宣布 `Generating avatar`、`Avatar ready`、`Couldn’t generate avatar`、`Generation cancelled`，不能随 spinner frame 重复朗读。
- Mac failure 将焦点放错误摘要；success 将焦点放 `Use avatar`；关闭后回到触发按钮。
- iOS 支持 Dynamic Type、Increase Contrast、Reduce Motion、Reduce Transparency；所有交互目标至少 44×44pt。

### 8.8 失败分类、iOS P6 修复与文案

客户端保留 status/category，不把所有错误压成一个字符串：

| 类别 | 用户文案 | 操作 |
| --- | --- | --- |
| 无网络/连接失败 | `Couldn’t reach the gateway.` | Retry |
| 504/客户端 timeout | `Avatar generation took too long.` | Retry / Cancel |
| provider/bridge 502 | `The image provider couldn’t generate an avatar.` | Retry / Change style |
| 空/损坏图片、normalize 失败 | `The generated image couldn’t be used.` | Retry |
| 用户取消 | 不显示错误 | 返回表单 |
| 被新 operation 取代 | 不显示错误 | 旧结果静默丢弃，但当前 operation 正常可见 |

iOS 明确修复 P6：

- `onGenerate` 不再以 `String?` 同时表示 failure、cancel、gateway switch/superseded；改为 Core 可测试的 typed outcome，例如 `.success(dataURL)`、`.failure(category, message)`、`.cancelled`、`.superseded`。
- model 不再一边写 `lastError` 一边返回 `nil`；由 section 对 `.failure` 一次性设置 inline failure，并调用一次现有 `onError(message)`。当前父层可继续把它写入 `model.lastError` 形成全局 fallback toast，但不会双写/双 toast。
- `.cancelled` / `.superseded` 不调用 `onError`。
- upload 与 generation 进入同一 error presentation policy；不能再出现 upload 调 `onError`、generation 静默 return 的分叉。
- Retry 由用户显式触发。图片生成非幂等，不在 5xx/timeout 后后台盲重试。

### 8.9 真取消、Desktop 双层竞态保护与迟到结果

一个 editor 同时最多一个 avatar operation；每次 operation 同时拥有 `generationToken` 与 `requestId`。

#### Renderer：counter/token + request ID

`AgentsHubPanel` 持有单调递增 `avatarGenerationEpochRef` 和当前 operation request ID：

1. Generate/Retry 时先递增 epoch，生成 request ID，捕获二者并进入 generating。
2. Cancel、dialog close、Agent form close、gateway switch 或新 operation 开始时再次递增 epoch，使所有旧 closure 立即失效。
3. success、failure 和 `finally` 只有在“捕获 token 等于当前 epoch”且“request ID 等于当前 operation”时才可更新 candidate/error/busy。
4. 旧 `finally` 不能清掉新请求的 loading；旧 success 不能显示 toast 或写 draft；旧 failure 不得覆盖新状态。

counter 是本地生命周期保险，request ID 是跨 IPC 的 operation 身份；两者不是互斥备选。

#### Electron main：AbortController

- generate IPC input 增加 request ID；preload 另暴露 scoped cancel IPC。
- main 维护 `Map<requestId, AbortController>`；generate 时注册，完成/失败/取消后清理。
- HTTP signal 合并用户 controller 与现有 630 秒 timeout signal。
- cancel IPC 只 abort 匹配 request ID，不影响其他窗口/editor 请求。
- renderer 无需也不能跨 IPC 直接传递 `AbortSignal`；传稳定 request ID 才是可序列化契约。

#### iOS：Task/URLSession + request ID

- 保留 section 的 Task 与 request ID，flow state 移入 Core。
- toolbar `Cancel generation` 调 `Task.cancel()`；Swift concurrency cancellation 传播到 `URLSession.data(for:)`。
- 只有当前 request ID 的 success/failure 可以进入 candidate/failed；onDisappear、gateway switch 与新 operation 使用同一 cancel transition。

#### Gateway：request-lifetime abort

- `tool_image` 在 bridge run 启动后建立 request-lifetime guard，持有 bridge handle + run ID。
- handler future 因客户端断连/取消被 drop 时，guard 安排 `abort_run(run_id)`。
- 正常 success/正常 provider failure 在返回前 disarm；timeout 继续使用现有显式 abort，并避免 cleanup 产生错误副作用。
- 这样 Desktop fetch abort 与 iOS URLSession cancel 都能沿 HTTP lifetime 终止真实 agent run，而不只是停止等待结果。

即使 abort 已来不及阻止 response 返回，renderer/iOS 的 token/request ID guard 仍会丢弃迟到结果。因此“资源取消”和“结果归属”两层都必须存在。

## 9. API 与并发契约

### 9.1 Agent API：保持不变

- `POST /api/custom-agents`：strict create；ID 已存在为 409。
- `GET /api/custom-agents/{id}`：iOS edit 建立权威草稿时使用。
- `PUT /api/custom-agents/{id}`：必须带 `expected_updated_at`；缺失 400、已删除 404、token 移动 409 并返回 `current_updated_at`。
- 不添加 unconditional update，不在 409 后自动重放旧草稿。
- ID create 后 immutable；update 始终以 URL ID 为准。

### 9.2 图片 API：wire contract 保持不变

```http
POST /api/tools/image
{ "prompt": "...", "timeout_secs": 600 }
```

- 不新增公开 cancel route、job route、progress 字段或 request ID wire 字段。
- 变化只在 Gateway handler 的 request-lifetime cleanup。
- Desktop 内部 IPC 增加 request ID + cancel；这不是 Gateway public API。
- Mac/iOS 继续构造相同 prompt。新增无个人数据的 parity fixture，验证 8 个 style ID、label、prompt 与最终 composition 文本。

## 10. 影响面（预计实现文件）

### 10.1 Desktop / Mac

| 文件 | 预计变化 |
| --- | --- |
| `desktop/garyx-desktop/src/renderer/src/app-shell/components/AgentAvatarEditor.tsx` | style dialog 扩展为 choosing/generating/candidate/failed；Current/New、overlay、Retry、Use |
| `.../components/AgentFormDialog.tsx` | 新 trigger/replace/remove 呈现；把生成状态传给专用 preview；保持身份字段现状 |
| `.../components/AgentsHubPanel.tsx` | 显式 avatar flow；generation counter + request ID；cancel 与 stale-result guard |
| `.../components/agents-hub-helpers.ts` | style/prompt parity fixture 接口；本任务不改变 `deriveId` |
| `desktop/garyx-desktop/src/renderer/src/styles/agents-hub.css` | overlay、spinner/sweep、Current/New、crossfade、error、reduced-motion |
| `desktop/garyx-desktop/src/renderer/src/i18n/index.tsx` | Generate new、Use avatar、Retry、Cancel generation、分类错误文案 |
| `desktop/garyx-desktop/src/shared/contracts/desktop-api.ts` | generate request ID 与 scoped cancel IPC contract |
| `desktop/garyx-desktop/src/preload/index.ts` | 暴露 generate/cancel IPC |
| `desktop/garyx-desktop/src/main/index.ts` | 注册 scoped cancel handler |
| `desktop/garyx-desktop/src/main/agent-avatar.ts` | AbortController registry/signal；保留 prompt 与 256px normalization |
| focused renderer/main tests | 状态转换、counter、cancel、late success/failure/finally、candidate acceptance、style parity |

身份规则默认不改 Mac 表单；create 手动 override 的开放问题见 12.6。

### 10.2 iOS / GaryxMobileCore

| 文件 | 预计变化 |
| --- | --- |
| 新 `Sources/GaryxMobileCore/GaryxCustomAgentDraftRules.swift`（命名可调） | name → ID、create/edit immutable ID、validation、409 collision、payload projection |
| `Sources/GaryxMobileCore/GaryxGatewayAgentModels.swift` | typed avatar outcome/flow state；style/prompt parity |
| `Sources/GaryxMobileCore/GaryxGatewayClient.swift` | 增加 single-agent GET；保留 status category 与 Task cancellation |
| `App/GaryxMobile/GaryxMobileAgentsViews.swift` | Name 必填、ID 只读；原生 sheet；全部 avatar 状态；修复 generation nil/onError 分叉 |
| `App/GaryxMobile/GaryxMobileModel+AgentsWorkspaces.swift` | fresh edit fetch、strict 409/404 mapping、typed generation outcome；不再双写 lastError |
| `App/GaryxMobile/GaryxMobileAgentPickerComponents.swift` | 仅补头像换图 crossfade 能力；loading overlay 留在编辑器专用容器，不污染所有列表头像 |
| `App/GaryxMobile/GaryxMobileFormComponents.swift` | 如需要，仅增加通用 saving/field status slot；不放 Agent 专有规则 |
| `Tests/GaryxMobileCoreTests/GaryxCustomAgentDraftRulesTests.swift` | name → ID golden、逐字段、collision、immutable ID、OCC |
| `Tests/GaryxMobileCoreTests/GaryxMobileAvatarEditorStateTests.swift` | choosing/generating/candidate/failed/retry/cancel、typed error、迟到结果 |
| `Tests/GaryxMobileCoreTests/GaryxGatewayClientTests.swift` | GET、409 status、URLSession cancellation |

UIKit 图片 normalization 和 PhotosPicker 留在 App target；业务规则与状态进 Core。

### 10.3 Gateway

| 文件 | 预计变化 |
| --- | --- |
| `garyx-gateway/src/tool_image.rs` | request-drop guard；客户端断连时 abort bridge run；正常/timeout disarm/cleanup tests |
| `garyx-gateway/src/api.rs` / `custom_agents.rs` | 预计无语义变化；strict create/OCC 保持 |
| API tests | 保持 Agent 409/400/404/409；新增 tool-image client-cancel abort 测试 |

## 11. 验收标准（供后续实现与评审）

### 11.1 Headless / contract

- SwiftPM tests 覆盖第 7.7 节全部身份规则，无需启动 UI。
- TS/Swift style 与 prompt parity fixture 同时通过。
- 两端 avatar flow tests 覆盖所有状态、Retry、Cancel、late success/failure/finally 和 candidate acceptance。
- Desktop 测试证明 counter/request ID 与 AbortController 两层同时生效。
- Gateway 测试证明客户端取消后对应 run 被 abort；正常完成不误 abort；timeout 仍返回 504。
- Agent API 测试继续证明 strict POST 409、PUT missing token 400、deleted 404、stale token 409。

### 11.2 UI 行为

- iOS create 只输入 Name 即得到只读 ID；不存在可编辑 ID 输入框或 binding guard。
- iOS edit 改 Name 后 ID 保持；不存在“输入成功但保存后回滚”的假交互。
- 纯 CJK/全标点 Name 显示明确字段错误，不只让 Save 变灰。
- create 409 保持草稿并定位 Name；只有 derived ID 改变后 collision 清除；没有 suffix/overwrite。
- edit 409 保持草稿与头像，要求 Reload；不自动覆盖另一端修改。
- Mac/iOS 的 choosing、generating、candidate、failed、retry、cancel、use 序列一致。
- loading overlay 在 preview 上立即出现，包含遮罩、spinner、文字；可选 sweep 不承载进度。
- iOS style sheet 不在 Generate 时 dismiss；不使用 0.93 fraction 自定义 glass 表面。
- iOS generation failure 进入 section-local failed state并调用一次 `onError`；upload/generation 一致；cancel/superseded 不报错。
- 新图有 crossfade；Reduce Motion 时关闭 sweep/位移但状态仍清楚。
- 生成成功前 Current 不变；未点 Use 关闭子流程不替换。
- Cancel 后迟到结果不改 UI，对应 Gateway run 最终中止。
- Mac X/Escape/footer close、iOS Cancel generation、form teardown 都走统一取消语义。
- light/dark、Dynamic Type、VoiceOver/ARIA、Reduce Motion、Reduce Transparency、Increase Contrast 下可读；iOS targets ≥44pt。

## 12. 取舍、备选与开放问题

### 12.1 冲突时自动加 `-2`

**不选。** 自动 suffix 会让 `ID = deriveId(Name)` 不再严格成立，并在并发下形成多次猜测 POST；Mac 也没有实体 suffix 规则。若未来允许同名 Agent，应先定义显示名可重复与 ID 展示的全局模型。

### 12.2 成功立即替换 + Undo

**不选。** Undo 容易再次依赖短暂 toast，Current/New 也难比较。候选 + `Use avatar` 多一次明确点击，但把“生成成功”和“采用结果”正确分开。

### 12.3 关闭子表面、后台生成

**本次不选。** 后台生成需要持久 job、跨页面 operation host、通知、draft identity 和 edit OCC 合并策略；否则 Save/关闭表单时结果归属不清。若真实数据表明生成经常超过一分钟，再评估异步 job。

### 12.4 新增异步 image job API

备选是 `POST /api/tools/image-jobs` + `GET/DELETE /{job_id}` 或 SSE。它可跨页面恢复，但 provider 仍无百分比，且增加持久化、清理与授权面。本次 request lifetime + bridge abort 足够覆盖聚焦交互。

### 12.5 Gateway 统一持有 style catalog 与 avatar prompt

长期可考虑 avatar-specific API，让客户端只传 Name/style ID/custom prompt；但这会把通用 image tool 扩成产品领域接口并引入 catalog 版本兼容。本次先用 parity fixture 防漂移。

### 12.6 Mac create 的手动 ID override（开放问题）

当前 Mac 源码允许 create override，任务明确要求 iOS 不再手填 ID。终稿因此只在 iOS 删除输入，并逐字复制 Mac `deriveId`；代价是 Mac 仍能创建非派生 ID，iOS 不能。

若产品定义实际是“所有端都不允许手填 ID”，后续实现任务应同时删除 Mac 的 `agentIdTouched` 和 create ID input。这是合理的小型统一，但它改变 Mac 现行行为，不能在本设计中伪装为“Mac 已经如此”。无论是否删除，Mac derive 规则、edit ID immutable 与 Gateway strict 409 均不变。

### 12.7 shimmer/sweep 是否默认启用

主方案允许但不强制。先以遮罩 + spinner + 文字作为完整 loading；若 sweep 在真实设备上能增加“仍在工作”的感知且不显噪声，再默认开启。Reduce Motion 必须关闭。

## 13. 推荐实施顺序

1. 先落 Core identity rules + golden tests，再改 iOS Name/ID 表单与 strict 409/OCC UX。
2. 抽象两端 avatar flow state，以 fake generator 跑通 choosing → candidate/failure/retry/cancel。
3. 补 Desktop counter/request ID、scoped IPC AbortController 与 Gateway request-lifetime abort。
4. 接真实 `/api/tools/image`，完成 Current/New 与 Use avatar。
5. 做两端 loading/motion/accessibility 验收，最后跑 Agent strict create/conditional update 端到端场景。

这个顺序先固定可测试规则和状态，再接长耗时副作用；任何阶段都不把 POST 退化成 upsert，也不把 PUT 变成 unconditional write。

## 14. 采纳/拒绝台账

| # | 来源版本 | 裁决 | 终稿落点 | 理由 |
| --- | --- | --- | --- | --- |
| A1 | B 版 P6 / iOS failure 分析 | **采纳并校正表述** | 5.3、P6、8.8 | 源码证实 section 的 nil guard 不调用 `onError`，而 upload 会调用；同时 model 的确写 `lastError`，所以准确现状是“全局 toast 可能出现，但局部失败路径静默”。typed outcome + 单次 onError 是明确修复。 |
| A2 | B 版 Mac generation counter | **采纳并与 Codex request ID/abort 合并** | P5、8.9 | counter 能阻止旧 closure 写 UI，但不能停止资源；request ID 负责跨 IPC 身份，AbortController/Gateway guard 负责真取消。三者解决不同层次，必须同时存在。 |
| A3 | B 版 P1–P9 | **采纳缺失细节并源码校正** | 第 6 节 | 纳入 preview 无 loading、Mac 自动关 dialog、无 token、三段断裂、长等待等；P3 明确到 React/CSS/SwiftUI 赋值位置，P9 明确 `dismiss()` 早于 parent begin。 |
| A4 | B 版 loading overlay | **采纳** | 8.6、11.2 | 半透明遮罩 + spinner 提供稳定原位反馈；可选 shimmer/sweep 增强生命感。终稿补充文字、8 秒说明、Reduce Motion/Transparency 与两端样式落点。 |
| R1 | B 版 iOS `agentIdTouched + isProgrammaticIdChange` 手填 ID | **拒绝，采用 Codex 主稿** | 1.1、7.1–7.3、12.6 | 老板明确要求“填 Name 自动生成 ID”；只读纯派生删除整套 binding guard 复杂度。Mac create override 暂不改，并保留为跨端开放问题。 |
| R2 | B 版生成成功直接替换 | **拒绝，采用 Codex 候选确认制** | 8.2–8.5、12.2 | 生成式结果不可预期；Current/New + `Use avatar` 让生成与采用解耦，关闭子流程前不破坏当前草稿头像。 |
| R3 | B 版取消仅客户端丢弃/优先 token | **拒绝，采用真取消** | 8.9、9.2 | “取消 HTTP 不一定终止 agent run”正是必须补 Gateway request-lifetime abort 的原因。token 仍保留作迟到结果保险，但不能替代 Electron AbortController、URLSession cancel 与 bridge abort。 |
| R4 | B 版保留 `.fraction(0.93)` 自定义玻璃 sheet | **拒绝，采用原生 iOS 表面** | 8.5、P13 | 仓库 mobile UI 规则要求 grouped native management surface；`NavigationStack + Form + .medium/.large` 更适配键盘、Dynamic Type、iPad 与系统 dismiss 行为，也不发明新的 IA。 |
