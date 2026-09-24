#!/bin/bash
# MimicWX-Linux 容器启动脚本
# 启动顺序: D-Bus → VNC → AT-SPI2 → WeChat → noVNC → 密钥提取/验证 → MimicWX

set +e  # 不因单个命令失败而退出

# ============================================================
# 0) 系统服务 (root)
# ============================================================
mkdir -p /run/dbus
dbus-daemon --system --fork 2>/dev/null || true

# 允许 ptrace (GDB 密钥提取需要)
echo 0 > /proc/sys/kernel/yama/ptrace_scope 2>/dev/null || true

# 修复权限
chmod 666 /dev/uinput 2>/dev/null || true
chown -R wechat:wechat /home/wechat/.xwechat 2>/dev/null || true
chown -R wechat:wechat /home/wechat/mimicwx-linux 2>/dev/null || true
mkdir -p /home/wechat/.xwechat/crashinfo/attachments
chown -R wechat:wechat /home/wechat/.xwechat

# 确保 /tmp/.X11-unix 存在且权限正确
mkdir -p /tmp/.X11-unix
chmod 1777 /tmp/.X11-unix

# VNC 密码由 Compose secret 提供，避免写入镜像层或容器环境变量
VNC_PASSWORD_FILE="${VNC_PASSWORD_FILE:-/run/secrets/vnc_password}"
if [ ! -r "$VNC_PASSWORD_FILE" ]; then
  echo "[start.sh] [err] VNC password secret is missing"
  exit 1
fi
IFS= read -r VNC_PASSWORD < "$VNC_PASSWORD_FILE"
if [ ${#VNC_PASSWORD} -lt 6 ]; then
  echo "[start.sh] [err] VNC password must contain at least 6 characters"
  exit 1
fi
install -d -m 700 -o wechat -g wechat /home/wechat/.vnc
printf '%s\n' "$VNC_PASSWORD" | su -s /bin/bash wechat -c 'vncpasswd -f' \
  > /home/wechat/.vnc/passwd
chown wechat:wechat /home/wechat/.vnc/passwd
chmod 600 /home/wechat/.vnc/passwd
unset VNC_PASSWORD

# ============================================================
# 1-8) 用户空间服务 (wechat 用户)
# ============================================================
su - wechat << 'USEREOF'
  set +e  # 确保单个命令失败不会终止整个 heredoc
  export LANG=zh_CN.UTF-8
  export LANGUAGE=zh_CN:zh
  export LC_ALL=zh_CN.UTF-8

  # 1) D-Bus session
  eval $(dbus-launch --sh-syntax)
  export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
  export QT_ACCESSIBILITY=1

  # 2) VNC + XFCE 桌面 (带重试和错误日志)
  echo "[start.sh] 启动 VNC..."
  vncserver -kill :1 2>/dev/null || true
  sleep 1
  vncserver :1 -geometry 1280x720 -depth 24 -localhost no 2>&1 | tee /tmp/vnc_startup.log
  VNC_EXIT=${PIPESTATUS[0]}
  if [ "$VNC_EXIT" != "0" ]; then
    echo "[start.sh] [warn] VNC 首次启动失败 (exit=$VNC_EXIT), 清理后重试..."
    vncserver -kill :1 2>/dev/null || true
    rm -f /tmp/.X1-lock /tmp/.X11-unix/X1 2>/dev/null || true
    sleep 2
    vncserver :1 -geometry 1280x720 -depth 24 -localhost no 2>&1 | tee -a /tmp/vnc_startup.log
  fi
  export DISPLAY=:1
  sleep 3

  # 验证 VNC 是否真正启动
  if [ -e /tmp/.X11-unix/X1 ]; then
    echo "[start.sh] [ok] VNC 启动成功 (DISPLAY=:1)"
  else
    echo "[start.sh] [err] VNC 启动失败! 后续服务可能不可用"
    echo "[start.sh] VNC 日志:"
    cat /tmp/vnc_startup.log 2>/dev/null || true
  fi

  # 禁用 XFCE 屏保/锁屏/电源管理 (防止息屏)
  xset s off 2>/dev/null || true
  xset -dpms 2>/dev/null || true
  xset s noblank 2>/dev/null || true
  xfconf-query -c xfce4-screensaver -p /saver/enabled -s false 2>/dev/null || true
  xfconf-query -c xfce4-power-manager -p /xfce4-power-manager/dpms-enabled -s false 2>/dev/null || true
  xfconf-query -c xfce4-power-manager -p /xfce4-power-manager/blank-on-ac -s 0 2>/dev/null || true

  # 3) 清理 XFCE 自启的 AT-SPI2 (避免 bus 冲突)
  for _r in 1 2 3; do
    pkill -9 -f at-spi-bus-launcher 2>/dev/null || true
    pkill -9 -f at-spi2-registryd 2>/dev/null || true
    sleep 0.5
  done
  rm -f ~/.cache/at-spi/bus_1 ~/.cache/at-spi/bus 2>/dev/null || true
  sleep 1

  # 4) 启动唯一的 AT-SPI2 bus
  /usr/libexec/at-spi-bus-launcher &
  sleep 2

  # 5) 获取 AT-SPI2 bus 地址
  A11Y_ADDR=$(dbus-send --session --dest=org.a11y.Bus --print-reply \
    /org/a11y/bus org.a11y.Bus.GetAddress 2>/dev/null \
    | grep string | sed 's/.*"\(.*\)"/\1/')
  if [ -n "$A11Y_ADDR" ]; then
    export AT_SPI_BUS_ADDRESS="$A11Y_ADDR"
    echo "[start.sh] [ok] AT-SPI2 bus: $A11Y_ADDR"
  else
    echo "[start.sh] [warn] AT-SPI2 bus address not found"
  fi

  # 保存环境变量 (供 docker exec 使用, 用 echo 避免嵌套 heredoc)
  echo "export DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS" > ~/.dbus_env
  echo "export DISPLAY=$DISPLAY" >> ~/.dbus_env
  echo "export LANG=zh_CN.UTF-8" >> ~/.dbus_env
  echo "export LANGUAGE=zh_CN:zh" >> ~/.dbus_env
  echo "export LC_ALL=zh_CN.UTF-8" >> ~/.dbus_env
  echo "export QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1" >> ~/.dbus_env
  echo "export QT_ACCESSIBILITY=1" >> ~/.dbus_env
  [ -n "$AT_SPI_BUS_ADDRESS" ] && echo "export AT_SPI_BUS_ADDRESS=$AT_SPI_BUS_ADDRESS" >> ~/.dbus_env

  # 6) 启动微信 (写 PID 供 GDB 使用, 保留 stderr 日志)
  echo "[start.sh] 启动微信..."
  wechat --no-sandbox --disable-gpu > /tmp/wechat_stdout.log 2>&1 &
  WECHAT_PID=$!
  echo $WECHAT_PID > /tmp/wechat.pid
  echo "[start.sh] [ok] 微信已启动 (PID: $WECHAT_PID)"
  # 等待微信窗口就绪 (轮询替代固定 sleep, 最多 60 秒)
  echo "[start.sh] 等待微信窗口就绪..."
  for _wait in $(seq 1 30); do
    # 检查微信窗口 (替代 xdotool, 使用进程窗口检测)
    if pgrep -x wechat >/dev/null 2>&1 && \
       xprop -root _NET_CLIENT_LIST 2>/dev/null | grep -q "0x"; then
      echo "[start.sh] [ok] 微信窗口已就绪 (${_wait}x2s)"
      break
    fi
    sleep 2
  done

  # 验证微信是否存活
  if kill -0 $WECHAT_PID 2>/dev/null; then
    echo "[start.sh] [ok] 微信进程存活"
  else
    echo "[start.sh] [err] 微信进程已退出! 日志:"
    cat /tmp/wechat_stdout.log 2>/dev/null | tail -20
  fi

  # 7) noVNC
  echo "[start.sh] 启动 noVNC..."
  websockify --web /usr/share/novnc 6080 localhost:5901 &
  echo "[start.sh] [ok] noVNC 已启动"

  # 环境变量已保存到 ~/.dbus_env (供 MimicWX 使用)
USEREOF

# ============================================================
# 7.5) 数据库密钥生命周期 (root)
# ============================================================
# WeChat 4.1+ 不再在内存保留 raw key。首次使用通过 GDB 捕获一次
# passphrase，之后按每个数据库的 salt 自动 PBKDF2 派生并 HMAC 验证。
WECHAT_VERSION=$(dpkg-query -W -f='${Version}' wechat 2>/dev/null || true)
echo "[extract_key] 微信版本: ${WECHAT_VERSION:-unknown}"
case "$WECHAT_VERSION" in
  4.0.*)
    echo "[extract_key] 使用旧版内存扫描提取器"
    python3 -u /usr/local/bin/extract_key_legacy.py \
      > /tmp/extract_key.log 2>&1 || true
    ;;
  *)
    echo "[extract_key] 使用微信 4.1+ 兼容提取器"
    python3 -u /usr/local/bin/extract_key_compat.py once --capture-timeout 600 \
      > /tmp/extract_key.log 2>&1 || true
    setsid python3 -u /usr/local/bin/extract_key_compat.py monitor --interval 60 \
      >> /tmp/extract_key.log 2>&1 &
    # 保持一个断点监听器待命；未来重新登录导致 passphrase 轮换时自动捕获。
    setsid python3 -u /usr/local/bin/extract_key_compat.py watch \
      --capture-timeout 86400 --retry-delay 10 \
      >> /tmp/extract_key.log 2>&1 &
    ;;
esac
echo "[extract_key] 启动校验完成, 详见 /tmp/extract_key.log"

# ============================================================
# 8) MimicWX (heredoc 之外运行, 保留 stdin 用于控制台命令)
# ============================================================
echo "=============================="
echo "MimicWX-Linux Ready!"
echo "noVNC: http://localhost:6080/vnc.html"
echo "API:   http://localhost:8899"
echo "=============================="

# 重启循环: 退出码 42 = 重启请求
while true; do
  # 通过 su -c 运行, 加载已保存的环境变量, 保留 stdin
  su - wechat -c '
    source ~/.dbus_env 2>/dev/null
    export RUST_LOG=mimicwx=info
    exec /usr/local/bin/mimicwx
  '
  EXIT_CODE=$?
  if [ "$EXIT_CODE" = "42" ]; then
    echo "[start.sh] [retry] MimicWX 重启中 (3秒后)..."
    sleep 3

    # ---- 重新验证/派生密钥 (不删除仍然有效的缓存) ----
    echo "[start.sh] [retry] 重新验证数据库密钥..."
    case "$WECHAT_VERSION" in
      4.0.*)
        python3 -u /usr/local/bin/extract_key_legacy.py >> /tmp/extract_key.log 2>&1 || true
        ;;
      *)
        python3 -u /usr/local/bin/extract_key_compat.py once --capture-timeout 600 \
          >> /tmp/extract_key.log 2>&1 || true
        ;;
    esac
    echo "[start.sh] [retry] 密钥验证完成"

    echo "[start.sh] [retry] 重新启动 MimicWX..."
    continue
  fi
  echo "[start.sh] MimicWX 已退出 (code=$EXIT_CODE)"
  break
done

echo "[start.sh] 容器退出"
