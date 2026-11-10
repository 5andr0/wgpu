//! Optional native Vulkan measurement; exact SPIR-V bytes bypass Naga re-import.
use super::{Result, controlled_producers, words};
use ash::{Entry, vk};
use std::{ffi::CStr, fs, path::Path, sync::Mutex};
const NAMES: [&str; 6] = [
    "original-controlled",
    "original-unmarked",
    "old-controlled",
    "old-unmarked",
    "fixed-controlled",
    "fixed-unmarked",
];
fn need(value: bool, message: &str) -> Result<()> {
    if value { Ok(()) } else { Err(message.into()) }
}
fn quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .chars()
            .flat_map(|c| match c {
                '"' => "\\\"".chars().collect(),
                '\\' => "\\\\".chars().collect(),
                c if c.is_control() => format!("\\u{:04x}", c as u32).chars().collect(),
                c => vec![c],
            })
            .collect::<String>()
    )
}
struct Case {
    code: Vec<u32>,
}
fn cases(args: &[String]) -> Result<(Vec<u8>, Vec<Case>)> {
    need(
        args.len() == 9,
        "gpu-plan|gpu-run INPUT16 SIX_SPIRV_FILES NEW_OUTPUT_DIRECTORY",
    )?;
    let input = fs::read(&args[1])?;
    need(
        input
            == [0x3f800001u32, 0x3f7ffffe, 0xbf800000, 0]
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
        "Unexpected generated input",
    )?;
    let mut result = Vec::new();
    for (i, path) in args[2..8].iter().enumerate() {
        need(
            fs::metadata(path)?.len() <= 1024 * 1024,
            "Shader exceeds one MiB",
        )?;
        let code = words(&fs::read(path)?)?;
        let expected = if i == 0 || i == 4 {
            vec![129, 133]
        } else {
            vec![]
        };
        need(
            controlled_producers(&code)? == expected,
            "Unexpected NoContraction producer set",
        )?;
        let instructions = super::instructions(&code)?;
        need(
            instructions
                .iter()
                .any(|x| x[0] & 65535 == 15 && x.get(1) == Some(&5)),
            "Expected compute entry",
        )?;
        need(
            instructions
                .iter()
                .any(|x| x[0] & 65535 == 16 && x.len() == 6 && x[2..] == [17, 1, 1, 1]),
            "Expected one invocation per workgroup",
        )?;
        result.push(Case { code });
    }
    Ok((input, result))
}
pub fn command(args: &[String]) -> Result<()> {
    let (input, cases) = cases(args)?;
    if args[0] == "gpu-plan" {
        println!(
            "{{\"device_created\":false,\"cases\":6,\"dispatches\":6,\"workgroups_per_case\":1,\"invocations_per_workgroup\":1,\"shared_input_bytes\":16,\"output_words\":6,\"input_audit_bytes\":16,\"shader_route\":\"native Vulkan exact SPIR-V; no Naga re-import\"}}"
        );
        return Ok(());
    }
    fs::create_dir(&args[8])?;
    unsafe { execute(&input, &cases, Path::new(&args[8])) }
}
type Messages = Mutex<Vec<(bool, String)>>;
unsafe extern "system" fn callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user: *mut std::ffi::c_void,
) -> vk::Bool32 {
    if !data.is_null() && !user.is_null() {
        if let Ok(mut messages) = unsafe { &*(user as *const Messages) }.lock() {
            if messages.len() < 128 {
                let pointer = unsafe { (*data).p_message };
                let bytes = if pointer.is_null() {
                    b"Missing validation message".as_slice()
                } else {
                    unsafe { CStr::from_ptr(pointer) }.to_bytes()
                };
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
                messages.push((
                    severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
                        || bytes.len() > 4096,
                    text,
                ));
            } else if messages.len() == 128 {
                messages.push((
                    true,
                    "Validation message count exceeded128; observation invalid".into(),
                ));
            }
        }
    }
    vk::FALSE
}
struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    bytes: usize,
}
struct InitialInstance {
    entry: Option<Entry>,
    instance: Option<ash::Instance>,
    debug: Option<ash::ext::debug_utils::Instance>,
    messenger: vk::DebugUtilsMessengerEXT,
    messages: Option<Box<Messages>>,
}
impl Drop for InitialInstance {
    fn drop(&mut self) {
        unsafe {
            if let Some(debug) = &self.debug {
                if self.messenger != vk::DebugUtilsMessengerEXT::null() {
                    debug.destroy_debug_utils_messenger(self.messenger, None);
                }
            }
            if let Some(instance) = &self.instance {
                instance.destroy_instance(None);
            }
        }
    }
}
struct Context {
    _entry: Entry,
    instance: ash::Instance,
    device: ash::Device,
    debug: ash::ext::debug_utils::Instance,
    messenger: vk::DebugUtilsMessengerEXT,
    messages: Box<Messages>,
    buffers: Vec<Buffer>,
    shaders: Vec<vk::ShaderModule>,
    pipelines: Vec<vk::Pipeline>,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    commands: vk::CommandPool,
    fence: vk::Fence,
    pending: bool,
    output_dir: std::path::PathBuf,
}
impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            if self.pending {
                eprintln!("Pending Vulkan work: exiting without destroying in-flight owners");
                std::process::exit(2);
            }
            for pipeline in &self.pipelines {
                self.device.destroy_pipeline(*pipeline, None);
            }
            for shader in &self.shaders {
                self.device.destroy_shader_module(*shader, None);
            }
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.commands, None);
            self.device.destroy_descriptor_pool(self.pool, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.destroy_descriptor_set_layout(self.layout, None);
            for buffer in &self.buffers {
                self.device.destroy_buffer(buffer.buffer, None);
                if buffer.memory != vk::DeviceMemory::null() {
                    self.device.free_memory(buffer.memory, None);
                }
            }
            self.device.destroy_device(None);
            self.debug
                .destroy_debug_utils_messenger(self.messenger, None);
            self.instance.destroy_instance(None);
            if let Ok(messages) = self.messages.lock() {
                let text = format!(
                    "[{}]\n",
                    messages
                        .iter()
                        .map(|(error, text)| format!(
                            "{{\"error\":{error},\"text\":{}}}",
                            quote(text)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                if let Err(error) = fs::write(self.output_dir.join("validation-final.json"), text) {
                    eprintln!("Final validation receipt failed: {error}");
                }
            }
        }
    }
}
impl Context {
    unsafe fn buffer(
        &mut self,
        memory: &vk::PhysicalDeviceMemoryProperties,
        bytes: &[u8],
        usage: vk::BufferUsageFlags,
    ) -> Result<usize> {
        unsafe {
            let buffer = self.device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes.len() as u64)
                    .usage(usage)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )?;
            self.buffers.push(Buffer {
                buffer,
                memory: vk::DeviceMemory::null(),
                bytes: bytes.len(),
            });
            let owned = self.buffers.len() - 1;
            let requirements = self.device.get_buffer_memory_requirements(buffer);
            need(
                requirements.size <= 1024 * 1024,
                "Unexpected per-buffer allocation requirement",
            )?;
            let index = (0..memory.memory_type_count)
                .find(|i| {
                    requirements.memory_type_bits & (1 << i) != 0
                        && memory.memory_types[*i as usize].property_flags.contains(
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
                })
                .ok_or("No coherent host-visible memory")?;
            let allocation = self.device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(index),
                None,
            )?;
            self.buffers[owned].memory = allocation;
            self.device.bind_buffer_memory(buffer, allocation, 0)?;
            let mapped = self.device.map_memory(
                allocation,
                0,
                bytes.len() as u64,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped as *mut u8, bytes.len());
            self.device.unmap_memory(allocation);
            Ok(owned)
        }
    }
    unsafe fn read(&self, i: usize) -> Result<Vec<u8>> {
        unsafe {
            let buffer = &self.buffers[i];
            let mapped = self.device.map_memory(
                buffer.memory,
                0,
                buffer.bytes as u64,
                vk::MemoryMapFlags::empty(),
            )?;
            let out = std::slice::from_raw_parts(mapped as *const u8, buffer.bytes).to_vec();
            self.device.unmap_memory(buffer.memory);
            Ok(out)
        }
    }
}
unsafe fn execute(input: &[u8], cases: &[Case], out: &Path) -> Result<()> {
    unsafe {
        let entry = Entry::load()?;
        let layers = entry.enumerate_instance_layer_properties()?;
        need(
            layers
                .iter()
                .any(|p| CStr::from_ptr(p.layer_name.as_ptr()) == c"VK_LAYER_KHRONOS_validation"),
            "Khronos validation layer required; no silent fallback",
        )?;
        let extensions = entry.enumerate_instance_extension_properties(None)?;
        need(
            extensions
                .iter()
                .any(|p| CStr::from_ptr(p.extension_name.as_ptr()) == ash::ext::debug_utils::NAME),
            "Debug utils extension required",
        )?;
        let messages = Box::new(Mutex::new(Vec::new()));
        let mut debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(callback))
            .user_data((&*messages as *const Messages).cast_mut().cast());
        let enabled_layers = [c"VK_LAYER_KHRONOS_validation".as_ptr()];
        let enabled_extensions = [ash::ext::debug_utils::NAME.as_ptr()];
        let app = vk::ApplicationInfo::default()
            .application_name(c"NoContraction authored demo")
            .api_version(vk::API_VERSION_1_1);
        let instance = entry.create_instance(
            &vk::InstanceCreateInfo::default()
                .application_info(&app)
                .enabled_layer_names(&enabled_layers)
                .enabled_extension_names(&enabled_extensions)
                .push_next(&mut debug_info),
            None,
        )?;
        let debug = ash::ext::debug_utils::Instance::new(&entry, &instance);
        let mut initial = InitialInstance {
            entry: Some(entry),
            instance: Some(instance),
            debug: Some(debug),
            messenger: vk::DebugUtilsMessengerEXT::null(),
            messages: Some(messages),
        };
        initial.messenger = initial
            .debug
            .as_ref()
            .unwrap()
            .create_debug_utils_messenger(&debug_info, None)?;
        let instance = initial.instance.as_ref().unwrap();
        let physical = instance
            .enumerate_physical_devices()?
            .into_iter()
            .find(|p| {
                instance.get_physical_device_properties(*p).device_type
                    == vk::PhysicalDeviceType::DISCRETE_GPU
            })
            .ok_or("No discrete Vulkan GPU; no software fallback")?;
        let properties = instance.get_physical_device_properties(physical);
        need(
            properties.api_version >= vk::API_VERSION_1_1,
            "Vulkan1.1 required",
        )?;
        let queue_family = instance
            .get_physical_device_queue_family_properties(physical)
            .iter()
            .position(|p| p.queue_count > 0 && p.queue_flags.contains(vk::QueueFlags::COMPUTE))
            .ok_or("No compute queue")? as u32;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let device = instance.create_device(
            physical,
            &vk::DeviceCreateInfo::default().queue_create_infos(&queues),
            None,
        )?;
        let queue = device.get_device_queue(queue_family, 0);
        let mut context = Context {
            _entry: initial.entry.take().unwrap(),
            instance: initial.instance.take().unwrap(),
            device,
            debug: initial.debug.take().unwrap(),
            messenger: initial.messenger,
            messages: initial.messages.take().unwrap(),
            buffers: vec![],
            shaders: vec![],
            pipelines: vec![],
            pool: vk::DescriptorPool::null(),
            layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            commands: vk::CommandPool::null(),
            fence: vk::Fence::null(),
            pending: false,
            output_dir: out.to_path_buf(),
        };
        initial.messenger = vk::DebugUtilsMessengerEXT::null();
        let name = CStr::from_ptr(properties.device_name.as_ptr()).to_string_lossy();
        fs::write(
            out.join("device.json"),
            format!(
                "{{\"backend\":\"native Vulkan SPIR-V\",\"name\":{},\"vendor\":{},\"device\":{},\"api_version\":{},\"driver_version\":{},\"validation_layer\":true,\"wgpu_or_Naga_import\":false}}\n",
                quote(&name),
                properties.vendor_id,
                properties.device_id,
                properties.api_version,
                properties.driver_version
            ),
        )?;
        let memory = context
            .instance
            .get_physical_device_memory_properties(physical);
        context.buffer(
            &memory,
            input,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
        )?;
        for _ in cases {
            context.buffer(
                &memory,
                &0x7fc01234u32.to_le_bytes(),
                vk::BufferUsageFlags::STORAGE_BUFFER,
            )?;
        }
        let audit = context.buffer(&memory, &[0; 16], vk::BufferUsageFlags::TRANSFER_DST)?;
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        context.layout = context.device.create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )?;
        let layouts = [context.layout];
        context.pipeline_layout = context.device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts),
            None,
        )?;
        let sizes = [vk::DescriptorPoolSize {
            ty: vk::DescriptorType::STORAGE_BUFFER,
            descriptor_count: 12,
        }];
        context.pool = context.device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(6)
                .pool_sizes(&sizes),
            None,
        )?;
        let set_layouts = [context.layout; 6];
        let sets = context.device.allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(context.pool)
                .set_layouts(&set_layouts),
        )?;
        for (i, case) in cases.iter().enumerate() {
            let info0 = [vk::DescriptorBufferInfo::default()
                .buffer(context.buffers[0].buffer)
                .range(16)];
            let info1 = [vk::DescriptorBufferInfo::default()
                .buffer(context.buffers[i + 1].buffer)
                .range(4)];
            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(sets[i])
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&info0),
                vk::WriteDescriptorSet::default()
                    .dst_set(sets[i])
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&info1),
            ];
            context.device.update_descriptor_sets(&writes, &[]);
            let shader = context.device.create_shader_module(
                &vk::ShaderModuleCreateInfo::default().code(&case.code),
                None,
            )?;
            context.shaders.push(shader);
            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(c"main");
            let pipelines = context
                .device
                .create_compute_pipelines(
                    vk::PipelineCache::null(),
                    &[vk::ComputePipelineCreateInfo::default()
                        .stage(stage)
                        .layout(context.pipeline_layout)],
                    None,
                )
                .map_err(|(partial, error)| {
                    context
                        .pipelines
                        .extend(partial.into_iter().filter(|p| *p != vk::Pipeline::null()));
                    format!("Compute pipeline: {error:?}")
                })?;
            context.pipelines.extend(pipelines);
        }
        context.commands = context.device.create_command_pool(
            &vk::CommandPoolCreateInfo::default().queue_family_index(queue_family),
            None,
        )?;
        let command = context.device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(context.commands)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )?[0];
        context.device.begin_command_buffer(
            command,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let initial = [vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(
                vk::AccessFlags::SHADER_READ
                    | vk::AccessFlags::SHADER_WRITE
                    | vk::AccessFlags::TRANSFER_READ,
            )];
        context.device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &initial,
            &[],
            &[],
        );
        for i in 0..cases.len() {
            context.device.cmd_bind_pipeline(
                command,
                vk::PipelineBindPoint::COMPUTE,
                context.pipelines[i],
            );
            context.device.cmd_bind_descriptor_sets(
                command,
                vk::PipelineBindPoint::COMPUTE,
                context.pipeline_layout,
                0,
                &[sets[i]],
                &[],
            );
            context.device.cmd_dispatch(command, 1, 1, 1);
        }
        context.device.cmd_copy_buffer(
            command,
            context.buffers[0].buffer,
            context.buffers[audit].buffer,
            &[vk::BufferCopy::default().size(16)],
        );
        let complete = [vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::HOST_READ)];
        context.device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::HOST,
            vk::DependencyFlags::empty(),
            &complete,
            &[],
            &[],
        );
        context.device.end_command_buffer(command)?;
        context.fence = context.device.create_fence(&Default::default(), None)?;
        let commands = [command];
        let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
        context.pending = true;
        if let Err(error) = context.device.queue_submit(queue, &submit, context.fence) {
            eprintln!("Submit failed; owners retained until process exit: {error:?}");
            std::process::exit(2);
        }
        if let Err(error) = context
            .device
            .wait_for_fences(&[context.fence], true, 30_000_000_000)
        {
            eprintln!("Fence failed; owners retained until process exit: {error:?}");
            std::process::exit(2);
        }
        context.pending = false;
        let audit_bytes = context.read(audit)?;
        fs::write(out.join("input-audit.bin"), &audit_bytes)?;
        need(audit_bytes == input, "Shared GPU input audit mismatch")?;
        let mut rows = Vec::new();
        for (i, name) in NAMES.iter().enumerate() {
            let bytes = context.read(i + 1)?;
            fs::write(out.join(format!("{name}.bin")), &bytes)?;
            let bits = u32::from_le_bytes(bytes.try_into().unwrap());
            need(bits != 0x7fc01234, "Output sentinel remained unwritten")?;
            rows.push(format!(
                "{{\"name\":{},\"bits\":\"{bits:08x}\"}}",
                quote(name)
            ));
        }
        let messages = context
            .messages
            .lock()
            .map_err(|_| "Validation message lock poisoned")?;
        let errors = messages.iter().filter(|x| x.0).count();
        fs::write(
            out.join("validation.json"),
            format!(
                "[{}]\n",
                messages
                    .iter()
                    .map(|(error, text)| format!("{{\"error\":{error},\"text\":{}}}", quote(text)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
        fs::write(
            out.join("result.json"),
            format!(
                "{{\"backend\":\"native Vulkan\",\"gpu_executed\":true,\"shared_input_audit_exact\":true,\"validation_errors\":{errors},\"unmarked_difference_required\":false,\"cases\":[{}]}}\n",
                rows.join(",")
            ),
        )?;
        need(
            errors == 0,
            "Vulkan validation errors; numeric observation invalid",
        )?;
        Ok(())
    }
}
