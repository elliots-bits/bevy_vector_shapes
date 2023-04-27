use bevy::{
    core_pipeline::core_2d::*,
    prelude::*,
    render::{
        render_phase::{DrawFunctions, RenderPhase},
        render_resource::*,
        renderer::RenderDevice,
        view::{ExtractedView, RenderLayers},
        Extract,
    },
    utils::FloatOrd,
};

use crate::render::*;

pub fn extract_instances_2d<T: Instanceable>(
    mut commands: Commands,
    entities: Extract<
        Query<(
            &T::Component,
            &GlobalTransform,
            &ComputedVisibility,
            Option<&Shape>,
            Option<&RenderLayers>,
        )>,
    >,
    mut events: Extract<EventReader<ShapeEvent<T>>>,
) {
    let mut instances = entities
        .iter()
        .filter_map(|(cp, tf, vis, flags, rl)| {
            if vis.is_visible() {
                Some((RenderKey::new(flags, rl), cp.instance(tf)))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    instances.extend(events.into_iter().map(|e| e.0));

    if !instances.is_empty() {
        commands.spawn(InstanceData::<T>(instances));
    }
}

fn spawn_buffers<T: Instanceable>(
    commands: &mut Commands,
    render_device: &RenderDevice,
    views: &Query<
        (Entity, Option<&RenderLayers>),
        (With<ExtractedView>, With<RenderPhase<Transparent2d>>),
    >,
    key: RenderKey,
    mut instances: Vec<T>,
) {
    if instances.is_empty() {
        return;
    }

    for (view_entity, render_layers) in views {
        if let Some(render_layers) = render_layers {
            if !render_layers.intersects(&key.render_layers) {
                continue;
            }
        }

        instances.sort_by_cached_key(|i| FloatOrd(i.distance()));

        // Workaround for an issue in the implementation of Chromes webgl ANGLE D3D11 backend
        #[cfg(target_arch = "wasm32")]
        if instances.len() == 1 {
            instances.push(T::null_instance());
        }

        let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("instance data buffer"),
            contents: bytemuck::cast_slice(instances.as_slice()),
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
        });
        commands.spawn(InstanceBuffer::<T> {
            view: view_entity,
            key,
            buffer,
            distance: instances[0].distance(),
            length: instances.len(),
            _marker: default(),
        });
    }
}

pub fn prepare_instance_buffers_2d<T: Instanceable>(
    mut commands: Commands,
    mut query: Query<&mut InstanceData<T>>,
    render_device: Res<RenderDevice>,
    views: Query<
        (Entity, Option<&RenderLayers>),
        (With<ExtractedView>, With<RenderPhase<Transparent2d>>),
    >,
) {
    for mut instance_data in &mut query {
        instance_data.sort_by_key(|(k, _i)| *k);

        let (key, instances) = instance_data.iter().fold(
            (instance_data[0].0, Vec::new()),
            |(key, mut instances), (next_key, instance)| {
                if *next_key == key {
                    instances.push(*instance);
                    (key, instances)
                } else {
                    spawn_buffers(
                        &mut commands,
                        render_device.as_ref(),
                        &views,
                        key,
                        instances,
                    );

                    (*next_key, vec![*instance])
                }
            },
        );

        spawn_buffers(
            &mut commands,
            render_device.as_ref(),
            &views,
            key,
            instances,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn queue_instances_2d<T: Instanceable>(
    transparent_2d_draw_functions: Res<DrawFunctions<Transparent2d>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<InstancedPipeline<T>>>,
    instanced_pipeline: ResMut<InstancedPipeline<T>>,
    pipeline_cache: Res<PipelineCache>,
    msaa: Res<Msaa>,
    instance_buffers: Query<(Entity, &InstanceBuffer<T>)>,
    mut views: Query<(&ExtractedView, &mut RenderPhase<Transparent2d>)>,
) {
    let draw_function = transparent_2d_draw_functions
        .read()
        .id::<DrawInstancedCommand<T>>();

    for (entity, buffer) in &instance_buffers {
        let (view, mut transparent_phase) = views
            .get_mut(buffer.view)
            .expect("View entity is gone during queue instances, oh no!");

        let mut key = InstancedPipelineKey::from_msaa_samples(msaa.samples());
        key |= InstancedPipelineKey::from_hdr(view.hdr);
        key |= InstancedPipelineKey::PIPELINE_2D;
        key |= InstancedPipelineKey::from_alpha_mode(buffer.key.alpha_mode.0);

        if !buffer.key.disable_laa {
            key |= InstancedPipelineKey::LOCAL_AA;
        }

        let pipeline = pipelines.specialize(&pipeline_cache, &instanced_pipeline, key);
        transparent_phase.add(Transparent2d {
            entity,
            pipeline,
            draw_function,
            sort_key: FloatOrd(buffer.distance),
            batch_range: None,
        });
    }
}
