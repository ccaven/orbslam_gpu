/*
TODO: 
 - Render blur texture using a normal vertex/fragment shader, samplers, two passes
 - Should get performance from 1.0ms to 0.5ms
*/

use std::sync::Arc;

use wgpu::{
    BufferUsages, TextureUsages
};

use tiny_wgpu::{
    Compute,
    ComputeProgram,
    BindGroupItem
};

pub struct OrbConfig {
    pub image_size: wgpu::Extent3d,
    pub max_features: u32,
    pub max_matches: u32
}

pub struct OrbParams {
    pub record_keyframe: bool
}

pub struct OrbProgram<'a> {
    pub config: OrbConfig,
    pub program: tiny_wgpu::ComputeProgram<'a>
}

impl OrbProgram<'_> {
    pub fn init(config: OrbConfig, compute: Arc<Compute>) -> Self {
        let mut program = ComputeProgram::new(compute);

        program.add_module("color_to_grayscale", wgpu::include_wgsl!("shaders/color_to_grayscale.wgsl"));
        program.add_module("gaussian_blur_x", wgpu::include_wgsl!("shaders/gaussian_blur_x.wgsl"));
        program.add_module("gaussian_blur_y", wgpu::include_wgsl!("shaders/gaussian_blur_y.wgsl"));
        program.add_module("corner_detector", wgpu::include_wgsl!("shaders/corner_detector.wgsl"));
        program.add_module("feature_descriptors", wgpu::include_wgsl!("shaders/feature_descriptors.wgsl"));
        program.add_module("feature_matching", wgpu::include_wgsl!("shaders/feature_matching.wgsl"));
        
        program.add_module("corner_visualization", wgpu::include_wgsl!("shaders/corner_visualization.wgsl"));
        program.add_module("matches_visualization", wgpu::include_wgsl!("shaders/matches_visualization.wgsl"));

        program.add_texture(
            "visualization",
            TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
            wgpu::TextureFormat::Rgba8Unorm,
            config.image_size
        );

        program.add_texture(
            "input_image", 
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC, 
            wgpu::TextureFormat::Rgba8Unorm, 
            config.image_size
        );
        
        program.add_sampler(
            "input_image_sampler",
            wgpu::SamplerDescriptor {
                label: None,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                lod_max_clamp: 1.0,
                lod_min_clamp: 0.0,
                compare: None,
                anisotropy_clamp: 1,
                border_color: None
            }
        );

        let half_size = wgpu::Extent3d { 
            width: config.image_size.width / 2, 
            height: config.image_size.height / 2, 
            depth_or_array_layers: 1
        };

        program.add_texture(
            "grayscale_image",
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            wgpu::TextureFormat::R16Float,
            half_size
        );

        program.add_texture(
            "gaussian_blur_x",
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            wgpu::TextureFormat::R16Float,
            half_size
        );

        program.add_texture(
            "gaussian_blur",
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            wgpu::TextureFormat::R16Float,
            half_size
        );

        program.add_buffer(
            "latest_corners",
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            (config.max_features * 8) as u64
        );

        program.add_buffer(
            "latest_corners_counter",
            BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            4
        );

        program.add_buffer(
            "previous_corners",
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            (config.max_features * 8) as u64
        );

        program.add_buffer(
            "previous_corners_counter",
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            4
        );

        program.add_buffer(
            "feature_matches",
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            (config.max_features * 4) as u64
        );

        program.add_buffer(
            "latest_descriptors",
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            (config.max_features * 8 * 4) as u64
        );

        program.add_buffer(
            "previous_descriptors",
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            (config.max_features * 8 * 4) as u64
        );

        // Stage 1: Color to grayscale
        {
            program.add_bind_group("color_to_grayscale", &[
                BindGroupItem::Sampler { label: "input_image_sampler" },
                BindGroupItem::Texture { label: "input_image" }
            ]);

            program.add_render_pipelines(
                "color_to_grayscale", 
                &["color_to_grayscale"], 
                &[("color_to_grayscale", ("vs_main", "fs_main"))],
                &[],
                &[Some(wgpu::TextureFormat::R16Float.into())], 
                &[]
            );
        }

        // Stage 2: Gaussian blur x
        {
            program.add_bind_group("gaussian_blur_x", &[
                BindGroupItem::Sampler { label: "input_image_sampler" },
                BindGroupItem::Texture { label: "grayscale_image" }
            ]);

            program.add_render_pipelines(
                "gaussian_blur_x", 
                &["gaussian_blur_x"], 
                &[("gaussian_blur_x", ("vs_main", "fs_main"))],
                &[],
                &[Some(wgpu::TextureFormat::R16Float.into())], 
                &[]
            );
        }
        
        // Stage 3: Gaussian blur y
        {
            program.add_bind_group("gaussian_blur_y", &[
                BindGroupItem::Sampler { label: "input_image_sampler" },
                BindGroupItem::Texture { label: "gaussian_blur_x" }
            ]);

            program.add_render_pipelines(
                "gaussian_blur_y", 
                &["gaussian_blur_y"],
                &[("gaussian_blur_y", ("vs_main", "fs_main"))],
                &[],
                &[Some(wgpu::TextureFormat::R16Float.into())], 
                &[]
            );
        }

        // Stage 4: Corner detection
        {
            program.add_bind_group("corner_detector", &[
                BindGroupItem::Texture { label: "grayscale_image" },
                BindGroupItem::StorageBuffer { label: "latest_corners", min_binding_size: 8, read_only: false },
                BindGroupItem::StorageBuffer { label: "latest_corners_counter", min_binding_size: 4, read_only: false }
            ]);

            program.add_compute_pipelines("corner_detector", &["corner_detector"], &["corner_detector"], &[]);
        }

        // Corner visualization
        {
            program.add_bind_group("corner_visualization", &[
                BindGroupItem::StorageTexture { label: "visualization", access: wgpu::StorageTextureAccess::WriteOnly },
                BindGroupItem::StorageBuffer { label: "latest_corners", min_binding_size: 8, read_only: true },
                BindGroupItem::StorageBuffer { label: "latest_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::Texture { label: "gaussian_blur" },
                BindGroupItem::Texture { label: "grayscale_image" },
            ]);

            program.add_compute_pipelines("corner_visualization", &["corner_visualization"], &["corner_visualization"], &[]);
        }

        // Stage 5: Feature descriptors
        {
            program.add_bind_group("feature_descriptors", &[
                BindGroupItem::Texture { label: "gaussian_blur" },
                BindGroupItem::StorageBuffer { label: "latest_corners", min_binding_size: 8, read_only: true },
                BindGroupItem::StorageBuffer { label: "latest_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "latest_descriptors", min_binding_size: 8 * 4, read_only: false },
                BindGroupItem::Texture { label: "grayscale_image" }
            ]);

            program.add_compute_pipelines("feature_descriptors", &["feature_descriptors"], &["feature_descriptors"], &[]);
        }

        // Stage 6: Feature matching
        {
            program.add_bind_group("feature_matching", &[
                BindGroupItem::StorageBuffer { label: "latest_descriptors", min_binding_size: 8 * 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "previous_descriptors", min_binding_size: 8 * 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "latest_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "previous_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "feature_matches", min_binding_size: 8, read_only: false }
            ]);
            
            program.add_compute_pipelines("feature_matching", &["feature_matching"], &["feature_matching"], &[]);
        }

        {
            program.add_bind_group("matches_visualization", &[
                BindGroupItem::StorageTexture { label: "visualization", access: wgpu::StorageTextureAccess::WriteOnly },
                BindGroupItem::StorageBuffer { label: "latest_corners", min_binding_size: 8, read_only: true },
                BindGroupItem::StorageBuffer { label: "latest_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "previous_corners", min_binding_size: 8, read_only: true },
                BindGroupItem::StorageBuffer { label: "previous_corners_counter", min_binding_size: 4, read_only: true },
                BindGroupItem::StorageBuffer { label: "feature_matches", min_binding_size: 8, read_only: true }
            ]);

            program.add_compute_pipelines("matches_visualization", &["matches_visualization"], &["matches_visualization"], &[]);
        }

        Self {
            config,
            program
        }
    }

    pub fn run(&self, params: OrbParams) {
        let mut encoder = self.program.compute.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: None
        });

        encoder.clear_buffer(&self.program.buffers["latest_corners_counter"], 0, None);
        encoder.clear_buffer(&self.program.buffers["feature_matches"], 0, None);

        // Grayscale image
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment { 
                        view: &self.program.texture_views["grayscale_image"], 
                        resolve_target: None, 
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store
                        },
                    })
                ],
                ..Default::default()
            });

            rpass.set_pipeline(&self.program.render_pipelines["color_to_grayscale"]);
            rpass.set_bind_group(0, &self.program.bind_groups["color_to_grayscale"], &[]);
            rpass.draw(0..3, 0..1);
        }

        // Gaussian blur x
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment { 
                        view: &self.program.texture_views["gaussian_blur_x"], 
                        resolve_target: None, 
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store
                        },
                    })
                ],
                ..Default::default()
            });

            rpass.set_pipeline(&self.program.render_pipelines["gaussian_blur_x"]);
            rpass.set_bind_group(0, &self.program.bind_groups["gaussian_blur_x"], &[]);
            rpass.draw(0..3, 0..1);
        }

        // Gaussian blur y
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment { 
                        view: &self.program.texture_views["gaussian_blur"], 
                        resolve_target: None, 
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store
                        },
                    })
                ],
                ..Default::default()
            });

            rpass.set_pipeline(&self.program.render_pipelines["gaussian_blur_y"]);
            rpass.set_bind_group(0, &self.program.bind_groups["gaussian_blur_y"], &[]);
            rpass.draw(0..3, 0..1);
        }

        // Corner detector
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });

            cpass.set_pipeline(&self.program.compute_pipelines["corner_detector"]);
            cpass.set_bind_group(0, &self.program.bind_groups["corner_detector"], &[]);
            cpass.dispatch_workgroups(
                (self.config.image_size.width / 2 + 7) / 8,
                (self.config.image_size.height / 2 + 7) / 8,
                1
            );
        }

        // Corner visualization
        {
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTextureBase { 
                    texture: &self.program.textures["input_image"], 
                    mip_level: 0, 
                    origin: wgpu::Origin3d::ZERO, 
                    aspect: wgpu::TextureAspect::All
                },
                wgpu::ImageCopyTextureBase { 
                    texture: &self.program.textures["visualization"], 
                    mip_level: 0, 
                    origin: wgpu::Origin3d::ZERO, 
                    aspect: wgpu::TextureAspect::All
                },
                self.config.image_size.clone()
            );
            
            // encoder.clear_texture(
            //     &self.program.textures["visualization"], 
            //     &wgpu::ImageSubresourceRange {
            //         aspect: wgpu::TextureAspect::All,
            //         base_mip_level: 0,
            //         mip_level_count: None,
            //         base_array_layer: 0,
            //         array_layer_count: None
            //     }
            // );

            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });

            cpass.set_pipeline(&self.program.compute_pipelines["corner_visualization"]);
            cpass.set_bind_group(0, &self.program.bind_groups["corner_visualization"], &[]);
            cpass.dispatch_workgroups(
                (self.config.max_features + 63) / 64,
                1,
                1
            );
        }

        // Feature descriptors
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });

            cpass.set_pipeline(&self.program.compute_pipelines["feature_descriptors"]);
            cpass.set_bind_group(0, &self.program.bind_groups["feature_descriptors"], &[]);
            cpass.dispatch_workgroups(
                (self.config.max_features + 63) / 64,
                1,
                1
            );
        }

        // Feature matching
        // We want the _best_ match for each feature
        // 
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });

            cpass.set_pipeline(&self.program.compute_pipelines["feature_matching"]);
            cpass.set_bind_group(0, &self.program.bind_groups["feature_matching"], &[]);
            cpass.dispatch_workgroups(
                self.config.max_features,
                (self.config.max_features + 63) / 64,
                1
            );
        }

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });

            cpass.set_pipeline(&self.program.compute_pipelines["matches_visualization"]);
            cpass.set_bind_group(0, &self.program.bind_groups["matches_visualization"], &[]);
            cpass.dispatch_workgroups(
                (self.config.max_features + 63) / 64,
                1,
                1
            );
        }

        if params.record_keyframe {
            encoder.copy_buffer_to_buffer(
                &self.program.buffers["latest_corners"], 
                0, 
                &self.program.buffers["previous_corners"], 
                0, 
                self.program.buffers["previous_corners"].size()
            );

            encoder.copy_buffer_to_buffer(
                &self.program.buffers["latest_corners_counter"], 
                0, 
                &self.program.buffers["previous_corners_counter"], 
                0, 
                self.program.buffers["previous_corners_counter"].size()
            );

            encoder.copy_buffer_to_buffer(
                &self.program.buffers["latest_descriptors"], 
                0, 
                &self.program.buffers["previous_descriptors"], 
                0, 
                self.program.buffers["previous_descriptors"].size()
            );
        }

        self.program.compute.queue.submit(Some(encoder.finish()));
    }
}