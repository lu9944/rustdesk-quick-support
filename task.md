# 任务描述
我需要做一个rustdesk-client的 tauri版本的客户端。

# 提示
1. `rustdesk`项目的源码路径是`/Users/lu/code/rust/rustdesk-dev/rustdesk`，可以研究是怎么被控的。
2. `rustdesk-QS`项目的源码路径是`/Users/lu/code/rust/rustdesk-dev/rustdesk_QS`，这个项目是已经实现了纯被控的功能，但是版本太老了，我需要使用`tauri`技术实现，做个更直观的界面，类似`TeamViewer QuickSupport`这个一样。
3. `rustdesk-server`项目的源码路径是`/Users/lu/code/rust/rustdesk-dev/rustdesk-server`，这个是官方的远程服务器实现源码，可以参考这个源码，编写远程服务器。

# 要求
1. 实现代码需要写到项目`/Users/lu/code/rust/rustdesk-dev/rustdesk-client`这里。
2. 使用`tauri`这个框架，实现gui编写，具体布局参考`TeamViewer QuickSupport`。
3. 只需要实现被控功能即可，其余类似服务器ID，密钥，socks5代理设置，写到`.env`这个环境变量里面，启动调试或者构建安装包的时候，直接把这些内置到被控客户端，客户无需填写。
4. 需要支持多端编译，交叉编译出windows，linux和mac端，需要写一个`build-all.sh`交叉编译脚本。