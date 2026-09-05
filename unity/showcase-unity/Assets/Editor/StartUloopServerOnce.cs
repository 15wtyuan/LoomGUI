// uloop MCP server 命令行拉起（AI 验收会话辅助）：Unity -executeMethod 只认 0 参静态方法，
// 而 McpServerController.StartServer(int port = -1) 带可选参会被拒（"has 1 arguments"）——
// 本包装提供 0 参入口，配合
//   Unity.exe -projectPath <proj> -executeMethod Showcase.EditorTools.StartUloopServerOnce.Run
// 在无头启动流程里把 server 拉起（UloopAutoStartServer 的 InitializeOnLoad 在本机
// 常不触发，见该文件头注释）。仅本机回环，无副作用；不需要时可删本文件。
using UnityEditor;

namespace Showcase.EditorTools
{
    internal static class StartUloopServerOnce
    {
        public static void Run()
        {
            try
            {
                var (running, port, _) = io.github.hatayama.uLoopMCP.McpServerController.GetServerStatus();
                if (running)
                {
                    UnityEngine.Debug.Log($"[StartUloopServerOnce] server already running on port {port}");
                    return;
                }
                io.github.hatayama.uLoopMCP.McpServerController.StartServer();
                UnityEngine.Debug.Log("[StartUloopServerOnce] uloop MCP server start requested");
            }
            catch (System.Exception e)
            {
                UnityEngine.Debug.LogWarning($"[StartUloopServerOnce] failed: {e.Message}");
            }
        }
    }
}
