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

也可以使用 `WECHAT_OA_APP_ID`、`WECHAT_OA_APP_SECRET` 环境变量，或通过全局 `--config` 参数指定配置文件。

## 常用命令

```bash
wechat-oa auth check
wechat-oa draft schema
wechat-oa draft validate ./my-draft
wechat-oa draft push ./my-draft
wechat-oa draft list
wechat-oa draft list --include-content
```

旧版本生成的草稿缓存没有绑定公众号账号。首次遇到这种缓存时，CLI 会拒绝自动归属；确认缓存属于当前账号后使用 `--legacy-state adopt`，或使用 `--legacy-state discard` 删除旧缓存并重新上传素材。

低级接口位于 `wechat-oa api` 下。执行 `wechat-oa --help` 或子命令的 `--help` 查看完整参数。
