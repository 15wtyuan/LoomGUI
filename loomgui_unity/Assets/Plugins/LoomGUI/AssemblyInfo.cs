// 暴露 LoomGUI.Bindings 的 internal（csbindgen 生成的 Native / StageHandle）给 LoomGUI.Runtime + LoomGUI.Tests。
// csbindgen 默认生成 internal 类型；LoomStage（LoomGUI.Runtime）+ 路由测（LoomGUI.Tests，v1c.4-T8 BuildStage）
// 需跨程序集调用 Native.loomgui_stage_new/load_html/free。
using System.Runtime.CompilerServices;

[assembly: InternalsVisibleTo("LoomGUI.Runtime")]
[assembly: InternalsVisibleTo("LoomGUI.Tests")]
