# wechat-oa-cli

面向自动化和 AI Agent 的微信公众号 CLI。成功结果输出到 stdout，错误以 JSON 输出到 stderr。

## 安装

从 [GitHub Releases](https://github.com/sunsunsunai/wechat-oa-cli/releases) 下载对应平台的压缩包：

| 系统 | 文件 |
| --- | --- |
| macOS Apple Silicon | `wechat-oa-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `wechat-oa-x86_64-apple-darwin.tar.gz` |
| Linux x64 | `wechat-oa-x86_64-unknown-linux-gnu.tar.gz` |
| Windows x64 | `wechat-oa-x86_64-pc-windows-msvc.zip` |

macOS 和 Linux 解压后，将二进制安装到 PATH：

```bash
tar -xzf wechat-oa-<target>.tar.gz
mkdir -p ~/.local/bin
install -m 755 wechat-oa ~/.local/bin/wechat-oa
wechat-oa --version
```

Windows 解压 ZIP 后，将 `wechat-oa.exe` 所在目录添加到 PATH。发布页同时提供 `SHA256SUMS` 用于校验下载文件。

## 配置

默认配置文件位于系统用户配置目录下的 `wechat-oa-cli/config.json`：

```json
{
  "app_id": "your-app-id",
  "app_secret": "your-app-secret"
}
```

在 Unix 系统上必须限制权限：

```bash
chmod 600 /path/to/config.json
```

也可以使用 `WECHAT_OA_APP_ID`、`WECHAT_OA_APP_SECRET` 环境变量，或通过全局 `--config` 参数指定配置文件。Windows 当前无法可靠验证配置文件 ACL，因此必须使用环境变量，避免从可能被其他用户读取的文件加载 `AppSecret`。

## 认证与权限

这里有两种容易混淆的“认证”：

- **调用认证**：访问微信服务器接口需要 `AppID`、`AppSecret`，CLI 会自动获取、缓存和刷新 `access_token`。
- **公众号主体认证**：决定某个公众号是否拥有草稿、素材、数据分析等业务接口权限。CLI 无法绕过微信后台的权限限制，最终以公众号后台显示的接口权限为准。

`draft schema` 和 `draft validate` 是纯本地命令，不读取凭据，也不会访问微信服务器。其他远端命令均需要有效配置。

## 接口实现情况

本表覆盖当前仓库已经涉及或紧邻的接口，不代表微信公众号的全部 API。

| 领域 | 微信接口 | 调用认证 | 主体认证/后台权限 | Rust Client | CLI |
| --- | --- | --- | --- | --- | --- |
| 基础能力 | 获取 `access_token` | `AppID` + `AppSecret` | 获取 token 本身不等于拥有业务权限 | ✅ | ✅ `auth check` |
| 草稿 | [新增草稿](https://developers.weixin.qq.com/doc/subscription/api/draftbox/draftmanage/api_draft_add.html) | `access_token` | 通常需要对应草稿权限 | ✅ | ✅ 高级 `draft push`；底层 `api draft add` |
| 草稿 | [获取草稿详情](https://developers.weixin.qq.com/doc/subscription/api/draftbox/draftmanage/api_getdraft.html) | `access_token` | 通常需要对应草稿权限 | ✅ | ✅ `draft get` |
| 草稿 | [获取草稿列表](https://developers.weixin.qq.com/doc/subscription/api/draftbox/draftmanage/api_draft_batchget.html) | `access_token` | 通常需要对应草稿权限 | ✅ | ✅ `draft list` |
| 草稿 | [获取草稿总数](https://developers.weixin.qq.com/doc/subscription/api/draftbox/draftmanage/api_draft_count.html) | `access_token` | 通常需要对应草稿权限 | ✅ | ✅ `draft count` |
| 草稿 | 更新草稿 | `access_token` | 通常需要对应草稿权限 | ❌ | ❌ |
| 草稿 | 删除草稿 | `access_token` | 通常需要对应草稿权限 | ❌ | ❌ |
| 永久素材 | [新增永久图片素材](https://developers.weixin.qq.com/doc/subscription/api/material/permanent/api_addmaterial.html) | `access_token` | 需要对应素材权限 | ✅ | ✅ 底层命令；`draft push` 会自动调用 |
| 图文素材 | [上传图文消息内图片](https://developers.weixin.qq.com/doc/subscription/api/material/permanent/api_uploadimage.html) | `access_token` | 需要对应素材权限 | ✅ | ✅ 底层命令；`draft push` 会自动调用 |
| 永久素材 | 获取、统计、列表、删除素材 | `access_token` | 需要对应素材权限 | ❌ | ❌ |
| 临时素材 | 上传、获取临时素材 | `access_token` | 以后台权限为准 | 非当前目标 | ❌ |
| 数据分析 | [获取图文阅读数据](https://developers.weixin.qq.com/doc/subscription/api/wedata/news/api_getarticleread.html) | `access_token` | 需要对应数据权限 | ✅ | ❌（暂不开放 analytics） |
| 发布 | 发布草稿、查询发布状态 | `access_token` | 需要对应发布权限 | ❌ | ❌ |

`✅` 只表示本项目已经实现相应代码，不代表当前公众号一定拥有微信侧权限。个人主体、未认证账号和不同类型公众号可用的接口可能不同。

## 高级命令

高级命令面向 AI Agent 和日常工作流，负责本地文件解析、参数校验、素材上传、HTML 改写、缓存和不确定请求保护。

| 命令 | 是否访问微信 | 作用 |
| --- | --- | --- |
| `wechat-oa draft schema` | 否 | 输出本地 `draft.json` 的 JSON Schema 和示例 |
| `wechat-oa draft validate <PATH>` | 否 | 校验文章或贴图 manifest、正文和本地图片，不上传任何内容 |
| `wechat-oa draft push <PATH>` | 是 | 自动上传文章封面/正文图片或贴图图片，最后创建远端草稿 |
| `wechat-oa draft count` | 是 | 获取远端草稿总数 |
| `wechat-oa draft list` | 是 | 分页列出草稿，默认不返回正文；`--include-content` 可包含正文 |
| `wechat-oa draft get <MEDIA_ID>` | 是 | 获取指定草稿详情 |

推荐的 AI 工作流：先在本地生成完整草稿目录，执行 `draft validate`，确认无误后再执行 `draft push`。

```bash
wechat-oa draft schema
wechat-oa draft validate ./my-draft
wechat-oa draft push ./my-draft
```

`draft push` 会维护按公众号隔离的本地状态，用于复用已经上传且内容未变化的图片，并避免网络结果不明确时自动重复创建草稿。复用草稿前会向微信确认它仍然存在且内容指纹未变化；如果素材在微信后台被删除，或确实需要重新上传所有图片并创建新草稿，可使用 `--force`。遇到历史无账号缓存时，确认属于当前账号后使用 `--legacy-state adopt`，或使用 `--legacy-state discard` 删除旧缓存并重新上传。

### 本地草稿格式

普通文章使用 `article_type: "news"`。正文是 HTML；CLI 会上传封面和正文引用的本地图片，并自动改写图片 URL：

```json
{
  "schema_version": 1,
  "article_type": "news",
  "title": "Article title",
  "content_file": "content.html",
  "cover_file": "assets/cover.jpg"
}
```

贴图消息使用 `article_type: "newspic"`。`content_file` 必须是 UTF-8 纯文本，`image_files` 接受 1～20 张图片；CLI 会将它们上传为永久素材，再按原顺序创建贴图草稿：

```json
{
  "schema_version": 1,
  "article_type": "newspic",
  "title": "Picture post title",
  "content_file": "content.txt",
  "image_files": [
    "assets/01.jpg",
    "assets/02.jpg"
  ]
}
```

所有文件路径必须是相对于 `draft.json` 所在目录的路径，且不能逃逸到目录之外。普通文章中的图片也必须使用本地相对路径；高级工作流会拒绝远程图片 URL，避免微信过滤外部图片后生成缺图草稿。完整字段和限制以 `wechat-oa draft schema` 的机器可读输出为准。

## 底层命令

底层命令位于 `wechat-oa api`，尽量一对一映射微信接口，不负责组装完整草稿目录。它们适合高级调用方、调试和已有 `media_id` 的场景。

| 命令 | 作用 | 输入要求 |
| --- | --- | --- |
| `wechat-oa api material upload-image <PATH>` | 上传永久图片素材 | BMP、PNG、JPEG/JPG 或 GIF，且满足微信大小限制 |
| `wechat-oa api material upload-article-image <PATH>` | 上传正文内图片并返回微信 URL | PNG 或 JPEG/JPG，且满足微信大小限制 |
| `wechat-oa api draft add --input <PATH>` | 直接调用新增草稿接口 | 已解析的 JSON；图片必须已经替换为微信 URL/`media_id`，也可用 `-` 从 stdin 读取 |

例如从 stdin 创建草稿：

```bash
cat request.json | wechat-oa api draft add --input -
```

底层命令仍会做字段、长度、图片格式和不确定请求校验，但不会替你扫描 HTML、上传依赖图片或改写正文。

## 常用命令

```bash
wechat-oa auth check
wechat-oa draft schema
wechat-oa draft validate ./my-draft
wechat-oa draft push ./my-draft
wechat-oa draft list
wechat-oa draft list --include-content
```

所有成功结果写入 stdout，所有错误以 JSON 写入 stderr。默认输出紧凑 JSON，传入全局参数 `--pretty` 可格式化输出。执行 `wechat-oa --help` 或任意子命令的 `--help` 查看完整参数。
