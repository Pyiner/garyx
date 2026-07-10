# 线程存储真相源反转:SQLite 终局设计

Date: 2026-07-08
Tracking: #TASK-1864(关联 #TASK-1829 收官线)
Status: 设计稿,待 codex 设计评审

## 1. 背景与现状

### 1.1 现状架构

```
调用方(router / bridge / gateway routes / tasks / automations / workflows)
    ↓ Arc<dyn ThreadStore>(garyx-router/src/store.rs,Value 接口,6 方法)
RecentThreadProjectingStore(garyx-gateway/src/recent_thread_projection.rs)
    per-thread 异步锁;archived 写拒绝;写后派生 3 组投影(5 张表)
    纯存储职责,无事件发射(SSE 走 transcript 提交面,与本手术无交集)
    ↓ inner
FileThreadStore(garyx-router/src/file_store.rs)
    lock 文件(10s 超时/30s stale)+ 50 permit 信号量 + 45s mtime 缓存
    ↓ 原子写(tmp+rename),整档案 pretty-print JSON
~/.garyx/data/threads/k_<hex>.json          ← meta 真相源(现状)
~/.garyx/data/transcripts/k_<hex>.jsonl     ← 对话内容真相源(不变)

garyx-db.sqlite3:thread_meta / recent_threads / task_projection /
thread_channel_endpoints / thread_message_routes = 写时投影影子
+ 开机 warmup:rebuild_thread_indexes + 6 项 backfill/reconcile
```

### 1.2 实测数据(2026-07-07,办公机)

- 档案文件:**4090 个 / 1.2GB**。构成:3780 `thread::*`、294
  `meta::agent_discussion::*`、14 `tool::image::*`(合成运行线程,单个最大
  2.5MB)、1 `meta::known_channel_endpoints`、1 `cron::vibe-usage`。
  store 是通用 KV,不只存线程。
- `messages` 复印件占 **88.3% 的紧凑字节**(1.06GB/1.2GB);加上
  pretty-print 缩进摊销,实际文件占比 ≈95%+;大文件内占比 98%+。
- 文件大小分布:p50=105KB,p90=775KB,p99=1.86MB,max=8MB。
- 去掉 `messages` 后的 body:典型 1–4KB;最大 687KB(6 个 2026-05 之前的
  化石记录,`history` 字段还是 transcript 迁移前的整段快照,390–662KB);
  `pending_user_inputs` 最大 149KB(单个离群值)。
- **复印机今天仍在运转**:每个 run 末尾整体重建 `messages` 快照并截到
  ≤100 条(§3.2);8MB 的档案是 cap 落地前的遗产。
- SQLite 现状:garyx-db.sqlite3 18MB,`journal_mode=delete`(WAL 未开,
  #TASK-1829 Batch 3 未落地);行数 thread_meta=3777、recent_threads=1130、
  task_projection=1118、endpoints=53、message_routes=2984。
- transcripts:4.2GB / 3519 个 jsonl,内容真相,本手术不动。

### 1.3 税单(为什么必须做)

1. **写放大**:每次 meta 小改(delivery 时间戳、run 状态、标题)都全量
   read-parse-serialize-write 整档案。最热路径是 bridge 持久化 worker 的
   `save_streaming_partial`(garyx-bridge/src/multi_provider/persistence.rs:1317):
   **每批 assistant delta 一次整档案重写**(经 try_recv 合批)。p50 档案
   105KB、p99 1.86MB 的重写单位,换一个 34 字节的 `updated_at`。
2. **每 delivery 一次全库扫描**:`sync_endpoint_delivery_timestamp`
   (garyx-router/src/threads.rs:670)在每次投递后 `list_keys(None)` +
   逐档案 `get`+`set` 命中的每条记录。`bind_endpoint_to_thread` /
   `detach_endpoint_from_thread` 同型。这正是
   docs/agents/repository-contracts.md "Thread Queries Never Enumerate
   Record Files" 明令禁止、但尚未拆除的存量。
3. **开机对账税**:`spawn_gateway_sync_cache_warmup`
   (garyx-gateway/src/composition/app_state.rs:246)依次跑
   `rebuild_thread_indexes`(全量扫描重建 endpoint 索引)+
   recent_threads backfill/prune/active-reconcile + thread_meta backfill +
   task backfill/reconcile。全部因为"文件是真相、SQLite 是影子"——影子
   可能漂,漂了要对。5.5GB 启动内存峰值(已修)与 198k 条告警日志都是
   这套架构的历史产物。
4. **RMW 丢更新窗口**:几乎所有写都是 `get → 改字段 → set 整档案`。
   bridge 持久化路径(不持 router 锁)与 router 路径(持 router 全局锁)
   并发 RMW 同一记录时,后写者覆盖前写者改的字段——四层锁
   (lock 文件 / 投影 store per-thread 锁 / router 全局锁 / task per-thread
   锁)没有一层横跨 bridge-vs-router 的完整 get→set 周期。
5. **锁文件/缓存复杂度**:跨进程 lock 文件轮询 + stale 启发式 + mtime
   校验缓存,都是文件存储自带的税。

## 2. 目标与非目标

目标(用户拍板的终局):

- meta 真相源反转到 SQLite;对话内容真相源 = transcript jsonl(不变)。
- `messages` 复印件、档案 JSON 文件、写时投影影子、开机对账、
  `rebuild_thread_indexes` 全部退役。
- 每消息整档案读改写的写放大一并消除。
- 调用方零改动:`ThreadStore` trait 面不变,换实现。

非目标:

- 不动 transcript store / message ledger(内容面)。
- 不动 app-database.sqlite3(#TASK-1829 Batch 3 的 app_db 半边仍归 1829)。
- 不改 HTTP API 合同、desktop/mobile 客户端。
- 不做记录 schema 归一化(见 D1 的否决理由)、不清洗化石 `history`
  字段(原样带走,后续可选)。
- 不做多机同步;迁移是每台机器首启自动完成的本地手术。

## 3. 前置审计结论

### 3.1 审计②:ThreadStore 调用方 / 写频率 / 锁模型

Trait 面(store.rs:9-43):`get/set/delete/list_keys/exists/update`,
全 async,Value 进出。**`update()` 生产零调用方**——语义可自由收紧成
真正的行级原子合并。实现 5 个:File、InMemory、RecentThreadProjecting
(双写包装)+ 2 个测试替身。

写频率画像(全部经 trait,无旁路;逐点见审计原文):

| 频率档 | 调用点 | 现状成本 |
|---|---|---|
| 每 delta 批 | bridge `save_streaming_partial`(persistence.rs:1317) | 整档案重写 |
| 每出站消息 | `persist_outbound_message_id`(routing.rs:272) | 整档案重写 |
| 每次投递 | `persist_delivery_context`(delivery.rs:152)+ `sync_endpoint_delivery_timestamp` | 整档案重写 + 全库扫描 fan-out |
| 每 run 起止 | 终局持久化、runtime snapshot、task 状态翻转、trim | 整档案重写 ×N |
| 低频 | create/label/binding/task CRUD/workflow 标记 | 整档案重写 |
| 扫描型 | bind/detach/sync-delivery、`list_user_threads`、`rebuild_from_store`、counts(routes.rs:3442、dashboard.rs:29、api.rs:105/388、mcp/helpers.rs:426) | `list_keys`+逐个 `get` 物化全库 |

锁模型:四层(§1.3-4);投影 store 的 per-thread 锁只包住
`inner.set + project`,不包调用方的前置 `get`——bridge-vs-router 的
RMW 竞态靠"通常改不同字段"侥幸共存。构造/装配:
`runtime_assembler.rs:42` 建 FileThreadStore →
`app_bootstrap.rs:352` 包投影 store;bridge/router/history/cron 全部
拿包装后的句柄;task 投影 reader 以 `Arc::as_ptr` 注册在**外层** store 上。
**没有任何代码绕过 trait 直接摸 threads 目录**(message ledger 与
transcript 的同名 helper 写的是别的目录)——trait 是干净的手术缝。

### 3.2 审计①:legacy `messages` 消费方裁决

先弄清 `messages` 的真实身份:它**不是逐条追加的历史**,而是 bridge 在
run 末尾重建的**provider 会话快照**——
`save_thread_messages_with_terminal_control`(persistence.rs:1535-1592)
把旧快照去掉本 run 旧条目、追加本 run 消息、截到
`MAX_SESSION_MESSAGES = 100`(persistence.rs:18)后整体回写;
`trim_thread_history`(dispatch_state.rs:22,limit=
`DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT = 100`)在 dispatch 侧再兜一层。
8MB 档案是这两个 cap 落地前的遗产。实证:transcript jsonl 的
`type=message` 记录携带与 `messages` 条目**同构**的 message 值
(role/content/text/timestamp/metadata/agent 字段一致),
`ProviderMessage::from_value` 可直接解析——这是"换源即退役"的依据。
但**不是**逐字节等价的窗口:transcript 里 control 记录
(`RunControlRecord`,无 `role`,persistence.rs:299)与内容记录交织
(persistence.rs:1107),`tail()` 又是按记录数取尾
(thread_history/store.rs:1939),所以换源需要**专用读取器**:倒扫
transcript,跳过 control/internal/不可解析记录,取最后 100 条
`ProviderMessage` 并保序。不承诺 tool_use/tool_result 配对完整——旧
快照的 100 条 cap 本身就可能截断配对,新读取器只需不劣于现状。
存量缺口(codex 评审发现,已实证):**59/3559** 条有非空 messages 的
线程没有对应 transcript 文件(pre-transcript 时代遗留)——见 D5/D7
的回退与补建安排。另:messages 条目里随 `runtime_context` 复印了
`garyx_mcp_auth_token`,退役复印件同时消除这份令牌落盘拷贝(安全红利)。

任务描述预期的七消费方有 4 项被审计**证伪**:gateway api 历史端点早已
transcript 供数(`snapshot.combined_messages()`,`"messages": []` 是空态
字面量)、task_projection 命中全在 `#[cfg(test)]`、cron.rs 不读
(automation 消费方在 automation.rs)、router lib.rs 只是 re-export。
真实消费面比预期更宽,如下。

写方清单(8 处):persistence.rs:1411-1580 run 末尾快照重建(实质写方,
为读方 #1 而存在)、routes.rs:767 线程导入播种(`seed_imported_thread_
history`,cap=`IMPORTED_SESSION_SNAPSHOT_LIMIT=100`,同时双写 transcript
——退役时只删快照半边)、router/tasks.rs:321 任务体播种为首条 user
消息(与 `task.body` 重复)、dispatch_state.rs:22 trim、scrub.rs:53
legacy 团队字段合并**写入** messages、persistence.rs:1280 /
navigation.rs:681 / threads.rs:443 三处 `messages: []` 初始化。

读方裁决表(12 项,行末为换源):

| 读方 | 读什么 | 裁决 | 换源 |
|---|---|---|---|
| bridge `persisted_provider_messages_from_thread`(session_resolve.rs:148-184)→ `garyx_native_provider.rs:709` 冷启动播种 | 整个快照 → `ProviderMessage`,原生 LLM provider(Gpt/ClaudeLlm/GeminiLlm)每 run 附带、agent-loop 会话为空时(gateway 重启后)重建 ≤100 轮上下文 | **真需要内容**(最高优先;Claude Code/Codex 走 `sdk_session_id` 自有会话,不经此路) | 读 transcript 尾窗 `type=message` 记录,同构解析,字节级等价 |
| gateway 同名**重复实现**(agent_identity.rs:126-145,prepare.rs:183 调用) | 同上 | 同上;顺带消重 | 与上共用一个 transcript 实现 |
| bridge 快照重建读回(persistence.rs:1428) | 旧快照做 run-id 去重 | 写方自读,随写方退役 | —— |
| thread_meta 投影 `last_message_preview_for_role`(thread_meta_projection.rs:333-357) | 尾部 user/assistant 预览 ≤160 字符 → 3 个列 | 惯性——写时可派生 | 写点维护 `last_user_preview`/`last_assistant_preview` 小字段 |
| recent 投影同款(recent_thread_projection.rs:574-603) | 同上 → `last_message_preview` 列 | 惯性 | 同上小字段 |
| automation `last_thread_message_preview`(automation.rs:483-501) | 尾部预览 → 自动化线程视图 | 惯性 | 同上小字段 |
| routes `last_message_preview`(routes.rs:1371-1392,`thread_summary`) | 尾部预览 → 线程 create/update 响应 | 惯性 | 同上小字段 |
| navigation `fallback_thread_list_label`(navigation.rs:120-163) | 尾部摘要 48 字符做标签回退 | 惯性——**transcript 回退已存在**(`history.latest_message_text` 是第二选择) | 删 legacy 分支,transcript 回退升为首选 |
| router tasks `derive_title`(tasks.rs:1238-1261) | 首条 user 消息做标题回退 | 惯性——读回自己播种的 `task.body` 副本 | 直接读 `input.title`/`task.body` |
| gateway tasks `task_body_from_record`(tasks.rs:1472-1490) | `task.body` 缺失时回退首条 user 消息(legacy 任务) | 惯性 + legacy 兜底 | 读 `task` 字段;真 legacy 记录由导入时把首条 user 消息一次性补进 `task.body` |
| CLI `task_progress_messages`(garyx/src/commands/task.rs:871-902) | API 返回的 `thread.messages` 做任务进度补充行 | 惯性——transcript 历史已是主源并做去重 | 删补充分支;记录 API `thread` 载荷从此不再携带 messages(响应变小,合同兼容) |

另有纯结构惯性,最后拆:`garyx-models` 类型化载体
`ThreadRecord.messages`(thread_record.rs:122)与
`SessionEntry.messages`(session.rs:142,`#[serde(default)]`——键缺失
可安全解析,导入剥离无反序列化风险);测试面(persistence/
recent_projection/scrub/channel 等 30+ 处 fixture)随各批同步改。

结论:硬内容消费方只有 2 个(原生 provider 续聊、群转写快照×2 实现),
且都消费"有界尾窗"——经专用读取器过滤后的 transcript 尾窗(有 tail
cache,075be4dd 后为流式)是语义等价的替代源;`message_count` 早已迁到
`history.message_count`。其余全部是写时可派生的预览/回退或已迁走的
化石。`messages` 可 100% 退役,无需任何"新形态的复印件"。

### 3.3 审计③:档案字段全集 vs thread_meta 列差集

对 3778 个 thread 档案 + 全部 meta/tool/cron 键做了逐键频率普查
(top-level keys,400 样本外推 + 大字段全量核对):

**已被 thread_meta 列覆盖**:thread_id、workspace_dir、
thread_type(派生)、label→thread_label、agent_id、provider_type、
provider_key、selected_model{,_reasoning_effort,_service_tier}(派生)、
sdk_session_id、created_at、updated_at、message_count(读
`history.message_count`)、last_{user,assistant}_message /
last_message_preview(从 `messages` 反扫派生!)、recent_run_id(读
`history.recent_committed_run_ids` 尾)、active_run_id(探针)、
worktree→worktree_json、delivery_context(+6 个 camel/snake legacy
变体)→last_delivery_context_json、default_list_hidden(派生自
hidden/source/workflow_child_run_id)。

**不在任何投影列里、只活在档案 body 的字段**(退役档案文件时必须随
body 整体搬进 SQLite):`metadata`(开放 map,196/400)、
`pending_user_inputs`(379/400,bridge 队列)、
`provider_sdk_session_ids`(365/400,多 provider 会话映射)、
**`task`(134/400,ThreadTask 全量记录——任务的真相源就嵌在线程档案里,
task_projection 只是它的影子)**、`history`(transcript 检查点状态)、
`channel_bindings`(数组本体;endpoints 表只是索引)、
`outbound_message_ids`、origin 四件套(channel/account_id/from_id/
is_group)、thread_title_source、provider_thread_title、source/hidden/
exclude_from_recent/automation_*/workflow_*(6 个镜像字段)、
thread_mode、loop_*、worktree 全量、若干一次性
legacy 字段。**结论:字段面是开放 schema(55+ 个观测键,含双格式
legacy),逐列归一化不可行,必须整 body 作为文档列搬迁(D1)。**

## 4. 终局架构

```
调用方(零改动)
    ↓ Arc<dyn ThreadStore>
SqliteThreadStore(garyx-gateway,新)
    per-key 异步锁(沿袭)、archived 写拒绝(沿袭)
    写 = 单事务:upsert thread_records + 派生并 upsert 5 个投影表
    读 = 独立 reader 连接(WAL 快照,不被写阻塞)
    ↓
garyx-db.sqlite3(WAL)
    thread_records(key, body, …)      ← meta 真相源(新)
    thread_meta / recent_threads / task_projection /
    thread_channel_endpoints / thread_message_routes
        ← 同事务派生表(不再是"影子",漂移在结构上不可能)
~/.garyx/data/transcripts/…           ← 内容真相源(不变)
```

退役清单:`messages` 复印件及其追加器;档案 JSON 文件与 lock 文件;
FileThreadStore 生产装配(保留为迁移导入的读取器);
RecentThreadProjectingStore(职责折叠进新 store);
`rebuild_thread_indexes` + 全部 backfill/prune/reconcile 开机对账;
mtime 缓存;`projection_states` 版本对账(对 3 个线程投影而言)。

### 4.1 表结构

```sql
CREATE TABLE IF NOT EXISTS thread_records (
    key         TEXT PRIMARY KEY,   -- thread::* / meta::* / cron::* / tool::*
    body        TEXT NOT NULL,      -- 规范 JSON,永不含 `messages`
    updated_at  TEXT,               -- body.updated_at 镜像,便于排序/诊断
    recorded_at TEXT NOT NULL
) STRICT;
```

投影表 schema 不变(读路径零改动);变的是写入时机:从"包装层第二事务"
变成"store 写事务内"。

### 4.2 方法语义映射

| trait 方法 | SQLite 实现 | 语义变化 |
|---|---|---|
| `get` | reader 连接 `SELECT body`,serde 解析 | 无(scrub 迁移完成后退役 scrub) |
| `set` | writer 事务:upsert record;`thread::*` 键内联派生 5 投影 | 投影从"写后异步影子"变"同事务" |
| `update` | writer 事务:读 body→顶层合并→写回+投影 | 从"文件级 RMW"变真正原子合并(生产零调用方,无兼容包袱) |
| `delete` | writer 事务:删 record + 5 投影行 | 无 |
| `list_keys` | `SELECT key [WHERE key LIKE prefix]` | 无;残存扫描型调用方立刻从 1.2GB 物化降到 ~20MB |
| `exists` | `SELECT 1` | 无 |

## 5. 关键设计决策

### D1 文档列,不做列归一化

`thread_records.body` 存整条记录 JSON(剥离 `messages`)。否决"全字段
拆列":审计③证明字段面是 55+ 键的开放 schema(含 metadata 开放 map、
嵌入式 task 记录、双格式 legacy delivery 字段),trait 又是 Value 进出、
调用方普遍整体 RMW——拆列等于一次全字段映射迁移,风险面完全不必要。
可查询性由同事务投影列提供(它们从此可信),而不是 json_extract 索引
(否决:开放 schema 上的表达式索引脆且不可演进)。

### D2 投影同事务化,对账在结构上退役

`set`/`update`/`delete` 在**一个** SQLite 事务里完成 record 写 + 5 个
投影派生写。崩溃要么全有要么全无,投影不可能再漂;因此 backfill /
prune / active-reconcile / `rebuild_thread_indexes` / task 投影
`ensure_current` 的兜底 rescan(#TASK-1829 Batch 4 想搬走的那块)全部
失去存在理由,整链删除。派生逻辑复用现有三个纯函数
(`thread_meta_projection_from_thread_data_with_active_run` /
`recent_thread_draft_from_thread_data_with_active_run` / task draft),
不重写业务语义。**实现约束**(codex 评审⑤):现有
`replace_thread_meta_projection` / `replace_task_projection` 等 public
方法各自开事务(garyx_db/mod.rs:1390、task_forest.rs:256),不能拼装
——需在 `garyx_db` 内新增一个**一次持锁的复合事务方法**(写
`thread_records` + 5 投影,单 BEGIN/COMMIT),由 SqliteThreadStore
独占调用。RecentThreadProjectingStore 的三件职责——per-thread 锁、
archived 写拒绝、投影派生——整体折叠进 SqliteThreadStore,包装层
删除。若保留包装层(投影在第二事务),漂移窗口和对账税原样存在,
等于白做,否决。

### D3 落位 garyx-gateway,构造点移进 AppStateBuilder

trait 在 garyx-router,但 GaryxDbService、投影派生函数、archived
墓碑、active-run 探针全在 garyx-gateway——SqliteThreadStore 作为
gateway 模块实现 router trait(与 RecentThreadProjectingStore 同款
依赖方向,无新耦合)。`runtime_assembler.rs:42` 不再建 FileThreadStore;
builder 在 garyx_db 就绪处构造新 store(InMemory 仅存于测试注入路径
`with_thread_store`)。打开失败沿袭 garyx_db 现行策略:boot panic。
task 投影 reader 的 `Arc::as_ptr` 注册改挂新 store 的 Arc,机制不变。

### D4 连接架构:共享单写连接 + 独立读连接,WAL

- 打开时设 `journal_mode=WAL`、`synchronous=NORMAL`、
  `busy_timeout=5000`、`foreign_keys=ON`——即 #TASK-1829 Batch 3 的
  garyx-db 半边,由本手术吸收落地(app_db 半边仍归 1829)。
- **写**:走 GaryxDbService 现有的单 `Mutex<Connection>`,新增
  `spawn_blocking` 异步包装;record 写 + 投影写共一个事务、一次
  fsync(WAL 下 sub-ms)。否决"store 自开第二写连接":同库双写连接在
  delta 级写频下会把 workflow_events 等其它写路径推进 SQLITE_BUSY
  重试;单写连接 + WAL 无 BUSY、顺序可控。
- **读**:store 自持独立 reader 连接(WAL 快照读),`get/exists/
  list_keys` 不排在写锁后面——控制面读延迟与数据面写频解耦。
  实现细节:GaryxDbService 需保存 DB path 供 reader 二连(现只存
  连接);in-memory 测试库用 shared-cache URI 或降级单连接读,合同
  套件对两种形态都跑。
- 写成本核算:典型 body 1–4KB + 5 个投影 upsert,对比现状 105KB–8MB
  文件重写 + 目录 fsync;每 delta 批一次的写从毫秒级 IO 降到亚毫秒行写。

### D5 `messages` 先退役,店后搬家

消费方迁移(依审计①裁决表)在 store 切换**之前**独立成批:两个硬内容
消费方(原生 provider 续聊、群转写快照)换读**专用 transcript 会话
读取器**(§3.2:倒扫、滤 control、取尾 100 条 `ProviderMessage`),
并保留 **legacy 回退**——读取器结果为空且 record 有非空 `messages`
时回读旧快照(覆盖 59 条无 transcript 的存量线程),回退分支在
Batch 2 导入补建 transcript 后删除;预览/标签/标题类消费方改读写时
维护的小字段(`last_user_preview` / `last_assistant_preview` ≤160
字符,由 run 末尾终局持久化写点维护)或任务自身字段。然后**停止
写入**:快照重建、线程导入播种的快照半边(transcript 半边保留)、
任务体播种、trim、scrub 的 messages 合并、三处 `messages: []` 初始化
全部移除,最后拆 `garyx-models` 类型化载体字段。**Batch 1 不物理删除
任何存量 `messages`**——剥离只发生在 Batch 2 导入(见 D7)。收益立刻
兑现(档案停涨、每次重写单位缩到 sans-messages body、令牌拷贝不再
落盘),且与 store 切换互相独立、可分别回滚。legacy 任务的标题兜底由
导入时把首条 user 消息一次性补进 `task.body`。

### D6 非 thread 键同表存储

`meta::agent_discussion::*`(294 个,共 354KB)、
`meta::known_channel_endpoints`、`cron::*`、`tool::image::*` 全部作为
普通 record 进 `thread_records`;投影派生保持现有 `is_thread_key` 门。
registry 记录将来可并进 endpoints 表,不在本手术范围。

### D7 迁移:首启一次性流式导入,FileThreadStore 当读取器

backend 非 file 且迁移状态行缺失时,boot 内同步执行:
`FileThreadStore::list_keys(None)` → 逐 key `get()`(免费获得 legacy
双目录解析 + scrub)→ **transcript 补建**:record 的 `messages` 非空
且对应 transcript 文件缺失(实测 59/3559)时,先用现成的
`rewrite_from_messages` 原语(routes.rs:781,导入播种路径已在生产
使用)从旧快照重建 transcript → 剥 `messages` → 单事务写入 record +
投影。一次一条,峰值内存 = 单条记录(最大 8MB);1.2GB 流式读一遍,
预估秒级–几十秒,`migration_states` 行记录源文件数/导入数/跳过数/
补建 transcript 数。导入完成前不服务请求(避免半迁移脑裂;一次性成本
可接受)。多机(办公/家)各自首启自迁。补建完成后 transcript 对所有
线程完备,D5 的 legacy 回退分支即可删除。

### D8 双写过渡与读切换(用户拍板路径的落地化)

config `sessions.thread_store`: `file` | `sqlite`(双写)| `sqlite-only`
(Batch 2 新增的字段,现不存在),env `GARYX_THREAD_STORE` 可覆盖
(免改配置急回滚)。

**回滚语义边界**(codex 评审④):所谓回滚,指**同一新二进制内**切
后端 flag——镜像是热的,随切随用。跨 Batch 1 的**二进制降级**不在
承诺内:旧二进制的原生 provider 续聊只读 `session_data["messages"]`
(session_resolve.rs:148),而 Batch 1 之后新 run 不再写快照,降级会
让窗口期内活跃的原生 provider 线程丢续聊上下文(subprocess provider
即绝大多数线程不受影响)。此限制记录在案,靠 Batch 1 自身质量兜底
(它不删存量数据,行为面有全套回归)。

- **`sqlite`(过渡)**:SQL 为真相——写序 = SQL 事务提交成功后,
  best-effort 镜像写文件(经原 FileThreadStore,含 lock 文件语义);
  读走 SQL。镜像滞后于崩溃窗口的代价 = 回滚到 file 时丢最后一笔写,
  可接受;反向(文件先写)会把窗口转嫁给真相源,否决。镜像重写自然
  剥掉存量文件里的 `messages`(Batch 1 后已无消费方,回滚安全)。
  抽样双读比对(canonical JSON,忽略易变字段)计数器上报 debug 日志,
  做切换信心。
- **`sqlite-only`(终态)**:停镜像;开机对账链、投影包装、
  FileThreadStore 生产路径全部拆除;`~/.garyx/data/threads` 整目录
  rename 到 `~/.garyx/backups/threads-final-<date>/` 冷备(手工可删)。

### D9 缓存与锁:先做减法

- 不移植 45s mtime 缓存:reader 连接点读 2–50KB body + 解析在
  µs–亚 ms 级,先量化后再决定是否加 LRU(避免把旧税换成新税)。
- 沿袭 per-key 异步写锁(折叠自包装层):保证同 key 的
  record+投影事务不交错、archived 拒绝原子。
- bridge-vs-router 的 RMW 丢更新窗口:本手术先以行级事务把"写"原子化,
  **窗口的根治靠后续批次把热点 RMW 调用方迁到 `update()` 部分合并**
  (生产零调用方,语义可自由定义)——不在切换批里顺手做,防步子过大。

## 6. 分批实施计划

每批独立可验证、可提交、可回滚;Claude 实现批配 codex 评审任务
(`--notify current-thread`)。

### Batch 0 — WAL + async 包装(前置,吸收 1829-B3 的 garyx-db 半边)

garyx-db 打开加 WAL/NORMAL/busy_timeout;GaryxDbService 增
`spawn_blocking` 异步入口(本手术新增调用全走异步入口;存量同步调用点
的迁移仍归 #TASK-1829)。验收:全量测试绿;写压下 `/health` 与
`GET /api/tasks` P99 无回归(用 1829 的 bench 采法)。

### Batch 1 — `messages` 退役(消费方迁移 + 停止复印)

按审计①裁决表 12 项逐个迁移,建议子序:(a) 原生 provider 续聊与群
转写快照换读 transcript 尾窗,两份群快照实现顺带消重为一(等价性用
"快照 vs 重建"对照测试锁死,含 tool_use 配对与 internal 记录过滤);
(b) 预览/标签/标题类:写点补 `last_user_preview` /
`last_assistant_preview` 小字段,投影/automation/routes 摘要改读新字段
(legacy 档案回退反扫暂留,Batch 2 导入后删),navigation 删 legacy
分支(transcript 回退升为首选),任务标题/正文回退改读 `task` 字段,
CLI 任务进度删补充分支;(c) 停止全部 8 处写入点(线程导入只保留
transcript 半边);(d) 拆 `ThreadRecord.messages` /
`SessionEntry.messages` 类型化载体与 30+ 处测试 fixture。验收:全仓无
生产代码读写 record `messages`(迁移导入除外);原生 provider 多轮
续聊回归(真实 Gpt/ClaudeLlm 线程 gateway 重启后续聊上下文完整);
团队群聊 prompt 快照回归;新消息后 recent 列表预览正确;线程导入回归;
档案文件尺寸停涨。

### Batch 2 — SqliteThreadStore + 导入迁移(默认 file,可选 sqlite)

新表 + store 实现(同事务投影)+ 首启导入 + 双写镜像 + 抽样比对
计数器。**store 合同测试套件**同时跑 File/InMemory/Sqlite 三实现
(get/set/update/delete/list_keys/exists 语义等价);导入 fixture 覆盖
canonical/legacy sessions/pre-v2 文件名/scrub 字段/非 thread 键/
超大化石记录。验收:合同套件绿;办公机真实数据 opt-in 导入后,
`garyx thread list` / recent / task 列表逐项与 file 模式一致;
导入耗时与内存峰值记录在案。

### Batch 3 — 读切 SQL(默认 sqlite,双写维持)

默认值翻转;开机对账链对 sqlite 后端短路(代码暂留);soak 期观察
比对计数器与投影一致性。回滚 = `GARYX_THREAD_STORE=file`(镜像是热的)。
验收:重启后零 backfill 日志;`/api/threads`、endpoints、tasks 全绿;
比对计数器零漂移(或逐条解释)。

### Batch 4 — 停写文件 + 拆除(默认 sqlite-only)

删镜像写;拆 RecentThreadProjectingStore、开机对账链、
`rebuild_thread_indexes`、task `ensure_current` rescan、lock 文件逻辑;
FileThreadStore 降级为导入读取器;归档目录 rename。验收:全量测试绿;
重启即服务(无 warmup 扫描日志);冷启动到首个请求可服务的时间对比
记录;`~/.garyx/data/threads` 不再被打开(fs 观察)。

### Batch 5 — 收割(独立度量,各自成批)

1. 扫描型调用方改 SQL 查询:bind/detach/`sync_endpoint_delivery_timestamp`
   fan-out 改 endpoints 表定位 + 点写;`list_user_threads*`、
   `rebuild_from_store`、counts 改投影查询/`COUNT(*)`。
2. 热点 RMW 调用方迁 `update()` 部分合并(per-delta、per-delivery、
   outbound append),关死 bridge-vs-router 丢更新窗口。
3. 可选:化石 `history` 快照清洗、registry 记录并表。

## 7. 风险与对策

| 风险 | 对策 |
|---|---|
| 导入遗漏 legacy 形态(sessions/、pre-v2 文件名、损坏 JSON) | 用 FileThreadStore 本身当读取器(其解析/回退/scrub 逻辑已在生产验证多年);损坏文件计数跳过并保留原文件;导入后 count 对账 |
| 单写连接成为新瓶颈 | WAL 下行写亚毫秒;写全部 spawn_blocking,不占 runtime worker;Batch 0 先量基线,Batch 2/3 用同 harness 对比 |
| 投影派生函数有隐藏的"读 messages"依赖 | 审计③已点名 last_* 预览是唯一此类派生;Batch 1 先换源并以现网数据回归 |
| 回滚后文件镜像缺最后一笔写 | 双写顺序 SQL-first 的已知代价;镜像滞后窗口 = 单笔;回滚属急救路径,可接受并记录 |
| 双 gateway 并发开同一数据目录(历史上出现过) | SQLite WAL 跨进程语义本就强于 30s stale-lock 启发式;比对计数器在 soak 期暴露异常 |
| `Arc::as_ptr` reader 注册失配 | 注册点随构造点一起移动,gateway 组合测试覆盖 |
| 8MB 单条 body(未剥离前的 tool::image/化石) | 导入剥 messages 后最大 body 687KB;SQLite 单行远未及限制;STRICT + 长度断言 |
| 真相集中到单文件,损坏爆炸半径大于逐线程文件 | WAL+NORMAL 本就崩溃安全;归档目录留作最终冷备;后续可选 `sqlite3_backup` 周期快照(不阻塞本手术) |

## 8. 验证策略

- 单元/合同:三实现合同套件;同事务投影的崩溃原子性(事务中途注入
  失败,断言 record 与投影同进退);导入 fixture 全形态。
- 集成:`cargo test -p garyx-router / -p garyx-gateway / -p garyx-bridge
  --all-targets` + workspace;现有 gateway 测试**不改动**即通过
  (调用方零改动的机器验证)。
- 性能:#TASK-1829 的 control-plane bench 采法,三个断面
  (Batch 0 前后、Batch 3 前后、Batch 4 前后)记录 `/health`、
  `GET /api/tasks`、`GET /api/recent-threads` P50/P99 与
  `scheduler_lag_ms`;另记冷启动时间与首启导入耗时。
- 真机:办公机先行(本机数据即 1.2GB 全量样本),家机随版本自迁;
  `scripts/build-local-cli.sh` + 受管 gateway 重启 + `garyx status`。

## 9. 与 #TASK-1829 的边界

- 本手术吸收:1829-B3 的 garyx-db 半边(WAL+async 包装)、1829-B4 的
  task 投影 backfill 搬迁(以"同事务化后整链退役"的方式超额完成)。
- 仍归 1829:app_db 半边、workflow 定义列表缓存、inline 图片 tokio::fs、
  POST /api/tasks 同步链审计(B5)、transcript 深页读(B7 已落)。
