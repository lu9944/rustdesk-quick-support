const { invoke } = window.__TAURI__?.core ?? {};

if (!invoke) {
    document.addEventListener("DOMContentLoaded", () => {
        document.getElementById("statusText").textContent = "Tauri API not loaded";
        document.getElementById("deviceId").textContent = "Error";
        document.getElementById("password").textContent = "------";
    });
}

let deviceId = "--- --- ---";
let password = "------";

function formatId(id) {
    if (id.length >= 9) {
        return id.slice(0, 3) + " " + id.slice(3, 6) + " " + id.slice(6);
    }
    return id;
}

function updateDisplay(data) {
    deviceId = data.id;
    password = data.password;
    document.getElementById("deviceId").textContent = formatId(deviceId);
    document.getElementById("password").textContent = password;

    const statusDot = document.getElementById("statusDot");
    const statusText = document.getElementById("statusText");
    const statusMessage = document.getElementById("statusMessage");

    if (data.connected) {
        statusDot.className = "status-dot connected";
        statusText.textContent = "Remote session active";
        if (data.peer_name) {
            statusText.textContent += " - " + data.peer_name;
        }
        statusMessage.innerHTML = "<p>Your device is currently being controlled remotely.</p>";
    } else {
        statusDot.className = "status-dot waiting";
        statusText.textContent = "Waiting for connection";
        statusMessage.innerHTML = "<p>Allow remote control by sharing your ID and password with your supporter.</p>";
    }
}

async function refreshStatus() {
    try {
        const status = await invoke("get_status");
        updateDisplay(status);
    } catch (e) {
        console.error("Failed to get status:", e);
        document.getElementById("statusText").textContent = "Error: " + e;
    }
}

async function copyToClipboard(field) {
    let text = "";
    if (field === "deviceId") {
        text = deviceId;
    } else if (field === "password") {
        text = password;
    }

    try {
        await navigator.clipboard.writeText(text);
        showToast("Copied!");
    } catch (e) {
        const textarea = document.createElement("textarea");
        textarea.value = text;
        textarea.style.position = "fixed";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand("copy");
        document.body.removeChild(textarea);
        showToast("Copied!");
    }
}

function showToast(message) {
    let toast = document.querySelector(".copied-toast");
    if (!toast) {
        toast = document.createElement("div");
        toast.className = "copied-toast";
        document.body.appendChild(toast);
    }
    toast.textContent = message;
    toast.classList.add("show");
    setTimeout(() => {
        toast.classList.remove("show");
    }, 1500);
}

async function init() {
    try {
        const version = await invoke("get_version");
        document.getElementById("version").textContent = "v" + version;
    } catch (e) {
        console.error("Failed to get version:", e);
        document.getElementById("version").textContent = "err";
    }

    await refreshStatus();
    setInterval(refreshStatus, 5000);
}

window.addEventListener("DOMContentLoaded", init);
window.copyToClipboard = copyToClipboard;
