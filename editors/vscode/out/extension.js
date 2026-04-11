"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const path = __importStar(require("path"));
const vscode_1 = require("vscode");
const node_1 = require("vscode-languageclient/node");
let client;
async function activate(context) {
    const config = vscode_1.workspace.getConfiguration("elm-lint");
    if (!config.get("enable", true)) {
        return;
    }
    const serverPath = findServerPath(config.get("serverPath", ""));
    if (!serverPath) {
        vscode_1.window.showWarningMessage("elm-lsp binary not found. Install it or set elm-lint.serverPath in settings.");
        return;
    }
    const serverExecutable = {
        command: serverPath,
    };
    const serverOptions = {
        run: serverExecutable,
        debug: serverExecutable,
    };
    const clientOptions = {
        documentSelector: [{ scheme: "file", language: "elm" }],
        synchronize: {
            fileEvents: [
                vscode_1.workspace.createFileSystemWatcher("**/*.elm"),
                vscode_1.workspace.createFileSystemWatcher("**/elm-assist.toml"),
            ],
        },
    };
    client = new node_1.LanguageClient("elm-lint", "Elm Lint", serverOptions, clientOptions);
    await client.start();
}
async function deactivate() {
    if (client) {
        await client.stop();
        client = undefined;
    }
}
function findServerPath(configPath) {
    // 1. Explicit config setting.
    if (configPath) {
        return configPath;
    }
    // 2. Project-local node_modules.
    const workspaceFolders = vscode_1.workspace.workspaceFolders;
    if (workspaceFolders) {
        for (const folder of workspaceFolders) {
            const localPath = path.join(folder.uri.fsPath, "node_modules", ".bin", "elm-lsp");
            try {
                require("fs").accessSync(localPath, require("fs").constants.X_OK);
                return localPath;
            }
            catch {
                // Not found here, try next.
            }
        }
    }
    // 3. Fall back to PATH.
    return "elm-lsp";
}
//# sourceMappingURL=extension.js.map