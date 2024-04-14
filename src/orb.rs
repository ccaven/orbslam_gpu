/*

TODO:
 - Set up compute shader and workgroups to compute FAST features and log them into the buffer
 -

*/

use std::num::NonZeroU64;
use std::sync::Arc;

use crate::compute::{Compute, ComputeProgram};
use crate::buffers::{StagingStorageBufferPair, StorageStagingBufferPair};

pub struct OrbFeatureExtractorConfig {
    pub image_width: usize,
    pub image_height: usize,
    pub max_features: usize,
    pub threshold: u32
}

pub struct OrbFeatureExtractorParams {
    pub compute_matches: bool
}
pub struct OrbFeatureExtractor {
    config: OrbFeatureExtractorConfig,
    module: wgpu::ShaderModule,
    image: StagingStorageBufferPair,
    image_buffer: wgpu::Buffer,
    counter_reset_buffer: wgpu::Buffer,
    features: StorageStagingBufferPair,
    counter: StorageStagingBufferPair,
    feature_descriptors: StorageStagingBufferPair,
    previous_feature_descriptors: StorageStagingBufferPair,
    previous_counter: StorageStagingBufferPair,
    matches: StorageStagingBufferPair,
    match_counter: StorageStagingBufferPair,
    orb_pipeline: wgpu::ComputePipeline,
    brief_pipeline: wgpu::ComputePipeline,
    matching_pipeline: wgpu::ComputePipeline,
    bind_groups: Vec<wgpu::BindGroup>
}

impl OrbFeatureExtractor {
    pub fn write_image_buffer(&self, compute: Arc<Compute>, data: &[u8]) {
        compute.queue.write_buffer(&self.image_buffer, 0, data);
        compute.queue.submit([]);
    }

    pub fn get_features(&self, compute: Arc<Compute>) -> Option<(u32, Vec<u32>, Vec<u32>)> {
        let data = {
            let (feature_slice, feature_receiver) = self.features.map_async();
            let (index_slice, index_receiver) = self.counter.map_async();
            let (descriptor_slice, descriptor_receiver) = self.feature_descriptors.map_async();
            
            compute.device.poll(wgpu::Maintain::Wait);

            let Ok(Ok(_)) = pollster::block_on(feature_receiver.recv_async()) else { return None; };
            let Ok(Ok(_)) = pollster::block_on(index_receiver.recv_async()) else { return None; };
            let Ok(Ok(_)) = pollster::block_on(descriptor_receiver.recv_async()) else { return None; };

            let feature_data = feature_slice.get_mapped_range();
            let feature_data: &[u32] = bytemuck::cast_slice(&feature_data);
            let feature_data: Vec<u32> = feature_data.iter().map(|x| *x).collect();
            
            let descriptor_data = descriptor_slice.get_mapped_range();
            let descriptor_data: &[u32] = bytemuck::cast_slice(&descriptor_data);
            let descriptor_data: Vec<u32> = descriptor_data.iter().map(|x| *x).collect();

            let index_data = index_slice.get_mapped_range();
            let index_data: &[u32] = bytemuck::cast_slice(&index_data);
            let index_data = *index_data.iter().next().unwrap();
            
            Some((index_data, feature_data, descriptor_data))
        };

        self.feature_descriptors.staging.unmap();
        self.counter.staging.unmap();
        self.features.staging.unmap();

        data
    }

    pub fn get_matches(&self, compute: Arc<Compute>) -> Option<Vec<u32>> {
        let data = {
            let (matches_slice, matches_receiver) = self.matches.map_async();
            let (counter_slice, counter_receiver) = self.match_counter.map_async();

            compute.device.poll(wgpu::Maintain::Wait);

            let Ok(Ok(_)) = pollster::block_on(matches_receiver.recv_async()) else { return None };
            let Ok(Ok(_)) = pollster::block_on(counter_receiver.recv_async()) else { return None };
            
            let counter_data = counter_slice.get_mapped_range();
            let counter_data: &[u32] = bytemuck::cast_slice(&counter_data);
            let counter_data: u32 = *counter_data.iter().next().unwrap();

            let matches_data = matches_slice.get_mapped_range();
            let matches_data: &[u32] = bytemuck::cast_slice(&matches_data);
            let matches_data: Vec<u32> = matches_data[..(counter_data as usize)].iter().map(|x| *x).collect();


            Some(matches_data)
        };
        
        self.matches.staging.unmap();
        self.match_counter.staging.unmap();

        data
    }
}

impl ComputeProgram for OrbFeatureExtractor {
    type Config = OrbFeatureExtractorConfig;
    type Params = OrbFeatureExtractorParams;

    fn init(config: OrbFeatureExtractorConfig, compute: Arc<Compute>) -> OrbFeatureExtractor {
        let image_buffer_size = config.image_width * config.image_height;
        let feature_buffer_size = config.max_features * 4 * 2;
        let descriptor_buffer_size = config.max_features * 4 * 8;

        let module = compute.device.create_shader_module(wgpu::include_wgsl!("shaders/orb_features.wgsl"));

        let threshold_buffer = compute.device.create_buffer(&wgpu::BufferDescriptor {
            size: 4, // a size u32,
            label: Some("Threshold Buffer"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false
        });

        let image_buffer = compute.device.create_buffer(&wgpu::BufferDescriptor {
            size: image_buffer_size as u64,
            label: Some("Image Buffer"),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false
        });

        let image_size_buffer = compute.device.create_buffer(&wgpu::BufferDescriptor {
            size: 2 * 4,
            label: Some("Image Dimensions Buffer"),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false
        });

        let counter_reset_buffer = compute.device.create_buffer(&wgpu::BufferDescriptor {
            size: 4,
            label: Some("Counter Reset Buffer"),
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false
        });

        {
            compute.queue.write_buffer(
                &threshold_buffer, 
                0, 
                bytemuck::cast_slice(&[config.threshold as u32])
            );
            compute.queue.write_buffer(
                &image_size_buffer, 
                0, 
                bytemuck::cast_slice(&[config.image_width as u32, config.image_height as u32])
            );
            compute.queue.submit([]);
        }


        let image = StagingStorageBufferPair::new(&compute.device, image_buffer_size);
        let counter = StorageStagingBufferPair::new(&compute.device, 4);
        let features = StorageStagingBufferPair::new(&compute.device, feature_buffer_size);
        let feature_descriptors = StorageStagingBufferPair::new(&compute.device, descriptor_buffer_size);

        // TODO: add previous_features to bind group 3
        let previous_features = StorageStagingBufferPair::new(&compute.device, feature_buffer_size);
        let previous_feature_descriptors = StorageStagingBufferPair::new(&compute.device, descriptor_buffer_size);
        let previous_counter = StorageStagingBufferPair::new(&compute.device, 4);
        let matches = StorageStagingBufferPair::new(&compute.device, config.max_features * 4);
        let match_counter = StorageStagingBufferPair::new(&compute.device, 4);

        let bind_group_0_layout = compute.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 0,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 1,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(8).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 2,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Uniform,  
                        has_dynamic_offset: false, 
                        min_binding_size:Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                }
            ]
        });

        let bind_group_1_layout = compute.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 0,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: true },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 1,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Uniform,  
                        has_dynamic_offset: false, 
                        min_binding_size:Some(NonZeroU64::new(8).unwrap()) 
                    },
                    count: None
                }
            ]
        });

        let bind_group_2_layout = compute.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 0,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4 * 8).unwrap()) 
                    },
                    count: None
                }
            ]
        });

        let bind_group_3_layout = compute.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 0,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4 * 8).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 1,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 2,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::COMPUTE,
                    binding: 3,
                    ty: wgpu::BindingType::Buffer { 
                        ty: wgpu::BufferBindingType::Storage { read_only: false },  
                        has_dynamic_offset: false, 
                        min_binding_size: Some(NonZeroU64::new(4).unwrap()) 
                    },
                    count: None
                }
            ]
        });

        let orb_pipeline_layout = compute.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                &bind_group_0_layout,
                &bind_group_1_layout
            ],
            push_constant_ranges: &[]
        });

        let brief_pipeline_layout = compute.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                &bind_group_0_layout,
                &bind_group_1_layout,
                &bind_group_2_layout
            ],
            push_constant_ranges: &[]
        });

        let matching_pipeline_layout = compute.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[
                &bind_group_0_layout,
                &bind_group_1_layout,
                &bind_group_2_layout,
                &bind_group_3_layout
            ],
            push_constant_ranges: &[]
        });

        let orb_pipeline = compute.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Orb Pipeline"),
            layout: Some(&orb_pipeline_layout),
            module: &module,
            entry_point: "compute_orb"
        });

        let brief_pipeline = compute.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Brief Pipeline"),
            layout: Some(&brief_pipeline_layout),
            module: &module,
            entry_point: "compute_brief"
        });

        let matching_pipeline = compute.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Brief Pipeline"),
            layout: Some(&matching_pipeline_layout),
            module: &module,
            entry_point: "compute_matches"
        });

        // Contains the feature array and counter
        let bind_group_0 = compute.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group 0 a"),
            layout: &bind_group_0_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: counter.storage.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: features.storage.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: threshold_buffer.as_entire_binding()
                }
            ]
        });

        // Contains the image array and dimensions
        let bind_group_1 = compute.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_1_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: image_buffer.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: image_size_buffer.as_entire_binding()
                }
            ]
        });

        // Contains the BRIEF descriptor array
        let bind_group_2 = compute.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group 2 b"),
            layout: &bind_group_2_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: feature_descriptors.storage.as_entire_binding()
                }
            ]
        });

        // Contains the previous BRIEF descriptor array, counter, and the feature matches / counter pair
        let bind_group_3 = compute.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group 3 b"),
            layout: &bind_group_3_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: previous_feature_descriptors.storage.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: previous_counter.storage.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: matches.storage.as_entire_binding()
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: match_counter.storage.as_entire_binding()
                }
            ]
        });

        Self {
            config,
            module,
            image,
            image_buffer,
            counter_reset_buffer,
            features,
            feature_descriptors,
            previous_feature_descriptors,
            previous_counter,
            counter,
            matches,
            match_counter,
            orb_pipeline,
            brief_pipeline,
            matching_pipeline,
            bind_groups: vec![bind_group_0, bind_group_1, bind_group_2, bind_group_3],
        }
    }

    fn run(&mut self, params: OrbFeatureExtractorParams, compute: Arc<Compute>) {
        let mut encoder = compute.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: None
        });

        encoder.copy_buffer_to_buffer(
            &self.counter_reset_buffer,
            0,
            &self.counter.storage,
            0,
            4
        );

        encoder.copy_buffer_to_buffer(
            &self.counter_reset_buffer,
            0,
            &self.match_counter.storage,
            0,
            4
        );

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None
            });
            
            cpass.set_pipeline(&self.orb_pipeline);
            cpass.set_bind_group(0, &self.bind_groups[0], &[]);
            cpass.set_bind_group(1, &self.bind_groups[1], &[]);
            
            cpass.dispatch_workgroups(
                ((self.config.image_width + 7)/ 8) as u32,
                ((self.config.image_height + 7) / 8) as u32,
                1 as u32
            );

            cpass.set_pipeline(&self.brief_pipeline);
            cpass.set_bind_group(0, &self.bind_groups[0], &[]);
            cpass.set_bind_group(1, &self.bind_groups[1], &[]);
            cpass.set_bind_group(2, &self.bind_groups[2], &[]);

            cpass.dispatch_workgroups(
                ((self.config.max_features + 63) / 64) as u32,
                1 as u32,
                1 as u32
            );

            if params.compute_matches {
                cpass.set_pipeline(&self.matching_pipeline);
                cpass.set_bind_group(0, &self.bind_groups[0], &[]);
                cpass.set_bind_group(1, &self.bind_groups[1], &[]);
                cpass.set_bind_group(2, &self.bind_groups[2], &[]);
                cpass.set_bind_group(3, &self.bind_groups[3], &[]);

                cpass.dispatch_workgroups(
                    ((self.config.max_features + 7) / 8) as u32,
                    ((self.config.max_features + 7) / 8) as u32,
                    1 as u32
                );
            }
        }
        
        encoder.copy_buffer_to_buffer(
            &self.feature_descriptors.storage,
            0,
            &self.previous_feature_descriptors.storage,
            0,
            self.feature_descriptors.size_bytes as u64
        );

        encoder.copy_buffer_to_buffer(
            &self.counter.storage,
            0,
            &self.previous_counter.storage,
            0,
            self.previous_counter.size_bytes as u64
        );

        self.features.copy(&mut encoder);
        self.counter.copy(&mut encoder);
        self.feature_descriptors.copy(&mut encoder);

        if params.compute_matches {
            self.matches.copy(&mut encoder);
            self.match_counter.copy(&mut encoder);
        }

        compute.queue.submit([encoder.finish()]);
    }
}

