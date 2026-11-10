#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <MetalPerformanceShaders/MetalPerformanceShaders.h>
#import <MetalPerformanceShadersGraph/MetalPerformanceShadersGraph.h>
#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <math.h>
#include <limits.h>
#include <float.h>

// This bridge is built with ARC. Only model preparation reads host tensor bytes;
// execution aliases the caller's private Metal buffers throughout.
static void OQFail(NSString *format, ...) NS_FORMAT_FUNCTION(1, 2);
static void OQFail(NSString *format, ...) {
    va_list args;
    va_start(args, format);
    NSString *message = [[NSString alloc] initWithFormat:format arguments:args];
    va_end(args);
    @throw [NSException exceptionWithName:@"OQCatalogGraph" reason:message userInfo:nil];
}
static void OQCheck(BOOL condition, NSString *message) {
    if (!condition) OQFail(@"%@", message);
}
static void OQError(char *error, size_t capacity, NSString *message) {
    if (error && capacity) snprintf(error, capacity, "%s", message.UTF8String ?: "native graph error");
}
static MPSDataType OQType(NSInteger dtype) {
    switch (dtype) {
        case 1: return MPSDataTypeFloat32;
        case 2: return MPSDataTypeUInt8;
        case 6: return MPSDataTypeInt32;
        case 7: return MPSDataTypeInt64;
        default: OQFail(@"unsupported ONNX tensor dtype %ld", (long)dtype);
    }
    return MPSDataTypeInvalid;
}
static NSUInteger OQWidth(NSInteger dtype) {
    switch (dtype) {
        case 1: case 6: return 4;
        case 2: return 1;
        case 7: return 8;
        default: OQFail(@"unsupported ONNX tensor dtype %ld", (long)dtype);
    }
    return 0;
}
static NSUInteger OQCount(NSArray<NSNumber *> *shape) {
    OQCheck([shape isKindOfClass:NSArray.class] && shape.count <= 16, @"invalid tensor rank");
    NSUInteger count = 1;
    for (id dim in shape) {
        OQCheck([dim isKindOfClass:NSNumber.class] && [dim longLongValue] >= 0, @"tensor shape must be concrete and nonnegative");
        NSUInteger d = [dim unsignedIntegerValue];
        OQCheck(!d || count <= NSUIntegerMax / d, @"tensor element count overflow");
        count *= d;
    }
    return count;
}
static NSUInteger OQBytes(NSArray<NSNumber *> *shape, NSInteger dtype) {
    NSUInteger n = OQCount(shape), w = OQWidth(dtype);
    OQCheck(n <= NSUIntegerMax / w, @"tensor byte size overflow");
    return n * w;
}
static NSInteger OQAxis(NSInteger axis, NSUInteger rank) {
    if (axis < 0) axis += (NSInteger)rank;
    OQCheck(axis >= 0 && (NSUInteger)axis < rank, @"axis outside tensor rank");
    return axis;
}
static NSMutableArray<NSNumber *> *OQFill(NSUInteger count, NSInteger value) {
    NSMutableArray *result = [NSMutableArray arrayWithCapacity:count];
    for (NSUInteger i = 0; i < count; ++i) [result addObject:@(value)];
    return result;
}
static void OQAttributes(NSDictionary *attributes, NSArray<NSString *> *allowed) {
    OQCheck([attributes isKindOfClass:NSDictionary.class], @"attributes must be an object");
    for (NSString *key in attributes) {
        if (![allowed containsObject:key]) OQFail(@"unsupported attribute %@", key);
    }
}
static NSInteger OQInt(NSDictionary *a, NSString *key, NSInteger fallback) {
    id value = a[key];
    if (!value) return fallback;
    OQCheck([value isKindOfClass:NSNumber.class], @"integer attribute must be numeric");
    return [value integerValue];
}
static double OQFloat(NSDictionary *a, NSString *key, double fallback) {
    id value = a[key];
    if (!value) return fallback;
    OQCheck([value isKindOfClass:NSNumber.class], @"float attribute must be numeric");
    return [value doubleValue];
}
static NSString *OQString(NSDictionary *a, NSString *key, NSString *fallback) {
    id value = a[key];
    if (!value) return fallback;
    OQCheck([value isKindOfClass:NSString.class], @"string attribute must be a string");
    return value;
}
static NSArray<NSNumber *> *OQArray(NSDictionary *a, NSString *key, NSArray *fallback) {
    id value = a[key] ?: fallback;
    OQCheck([value isKindOfClass:NSArray.class], @"list attribute must be an array");
    for (id n in value) OQCheck([n isKindOfClass:NSNumber.class], @"list attribute must contain numbers");
    return value;
}

@interface OQCatalogValue : NSObject
@property(nonatomic, copy) NSArray<NSNumber *> *shape;
@property(nonatomic) NSInteger dtype;
@property(nonatomic, strong) NSData *data;
@property(nonatomic, strong) MPSGraphTensor *tensor;
@end
@implementation OQCatalogValue
@end

static OQCatalogValue *OQValue(NSArray *shape, NSInteger dtype, NSData *data, MPSGraphTensor *tensor) {
    OQBytes(shape, dtype);
    OQCheck(data != nil || tensor != nil, @"MPSGraph operation produced no tensor");
    if (data) OQCheck(data.length == OQBytes(shape, dtype), @"constant payload does not match shape and dtype");
    if (tensor) {
        OQCheck(tensor.dataType == OQType(dtype), @"MPSGraph inferred an unexpected tensor dtype");
        if (tensor.shape) {
            OQCheck(tensor.shape.count == shape.count, @"MPSGraph inferred an unexpected tensor rank");
            for (NSUInteger i = 0; i < shape.count; ++i) {
                if ([tensor.shape[i] longLongValue] >= 0 && ![tensor.shape[i] isEqual:shape[i]])
                    OQFail(@"MPSGraph inferred shape %@, expected %@", tensor.shape, shape);
            }
        }
    }
    OQCatalogValue *value = [OQCatalogValue new];
    value.shape = shape; value.dtype = dtype; value.data = data; value.tensor = tensor;
    return value;
}
static NSNumber *OQNumber(OQCatalogValue *v, NSUInteger index) {
    OQCheck(v.data != nil && index < OQCount(v.shape), @"constant value is unavailable");
    const uint8_t *p = (const uint8_t *)v.data.bytes + index * OQWidth(v.dtype);
    switch (v.dtype) {
        case 1: { float x; memcpy(&x, p, 4); return @(x); }
        case 2: return @(*p);
        case 6: { int32_t x; memcpy(&x, p, 4); return @(x); }
        case 7: { int64_t x; memcpy(&x, p, 8); return @(x); }
    }
    OQFail(@"unsupported constant dtype"); return nil;
}
static void OQStore(NSMutableData *data, NSInteger dtype, NSUInteger index, NSNumber *value) {
    uint8_t *p = (uint8_t *)data.mutableBytes + index * OQWidth(dtype);
    switch (dtype) {
        case 1: { float x = value.floatValue; memcpy(p, &x, 4); break; }
        case 2: { uint8_t x = value.unsignedCharValue; memcpy(p, &x, 1); break; }
        case 6: { int32_t x = value.intValue; memcpy(p, &x, 4); break; }
        case 7: { int64_t x = value.longLongValue; memcpy(p, &x, 8); break; }
        default: OQFail(@"unsupported constant dtype");
    }
}
static OQCatalogValue *OQNumbers(NSArray<NSNumber *> *numbers, NSArray *shape, NSInteger dtype) {
    OQCheck(numbers.count == OQCount(shape), @"constant element count mismatch");
    NSMutableData *data = [NSMutableData dataWithLength:OQBytes(shape, dtype)];
    for (NSUInteger i = 0; i < numbers.count; ++i) OQStore(data, dtype, i, numbers[i]);
    return OQValue(shape, dtype, data, nil);
}
static NSArray<NSNumber *> *OQNumbersOf(OQCatalogValue *v, BOOL integer) {
    OQCheck(v && v.data, @"shape/axes/resize input must be constant after shape folding");
    OQCheck(!integer || v.dtype == 6 || v.dtype == 7, @"shape/axes input must have integer dtype");
    NSMutableArray *values = [NSMutableArray arrayWithCapacity:OQCount(v.shape)];
    for (NSUInteger i = 0; i < OQCount(v.shape); ++i) [values addObject:OQNumber(v, i)];
    return values;
}
static OQCatalogValue *OQDescriptor(NSDictionary *desc, NSData *blob) {
    OQCheck([desc isKindOfClass:NSDictionary.class], @"missing tensor descriptor");
    NSArray *shape = desc[@"shape"];
    NSInteger dtype = [desc[@"dtype"] integerValue];
    NSUInteger bytes = OQBytes(shape, dtype);
    OQCheck([desc[@"offset"] isKindOfClass:NSNumber.class] && [desc[@"length"] isKindOfClass:NSNumber.class], @"missing constant blob range");
    OQCheck([desc[@"offset"] longLongValue] >= 0 && [desc[@"length"] longLongValue] >= 0, @"negative constant blob range");
    NSUInteger offset = [desc[@"offset"] unsignedIntegerValue], length = [desc[@"length"] unsignedIntegerValue];
    OQCheck(offset <= blob.length && length <= blob.length - offset && length == bytes, @"constant blob range/size mismatch");
    return OQValue(shape, dtype, [blob subdataWithRange:NSMakeRange(offset, length)], nil);
}
static MPSGraphTensor *OQTensor(MPSGraph *graph, OQCatalogValue *v) {
    OQCheck(v != nil, @"missing required tensor input");
    if(!v.tensor) {
        NSUInteger count=OQCount(v.shape);
        OQCheck(v.data && count>0, @"cannot materialize an empty or nonconstant tensor");
        if(v.shape.count==0) v.tensor=[graph constantWithScalar:OQNumber(v,0).doubleValue dataType:OQType(v.dtype)];
        else v.tensor = [graph constantWithData:v.data shape:v.shape dataType:OQType(v.dtype)];
        OQCheck(v.tensor != nil, @"MPSGraph constant allocation failed");
    }
    return v.tensor;
}
static OQCatalogValue *OQInput(NSArray *inputs, NSUInteger index, BOOL optional) {
    OQCatalogValue *v = index < inputs.count && inputs[index] != NSNull.null ? inputs[index] : nil;
    if (!optional) OQCheck(v != nil, @"missing required operator input");
    return v;
}
static NSArray *OQBroadcast(NSArray *a, NSArray *b) {
    NSUInteger rank = MAX(a.count, b.count);
    NSMutableArray *shape = OQFill(rank, 1);
    for (NSUInteger i = 0; i < rank; ++i) {
        NSInteger ad = i < rank - a.count ? 1 : [a[i - (rank - a.count)] integerValue];
        NSInteger bd = i < rank - b.count ? 1 : [b[i - (rank - b.count)] integerValue];
        OQCheck(ad == bd || ad == 1 || bd == 1, @"incompatible broadcast shapes");
        shape[i] = @(ad == 1 ? bd : ad);
    }
    return shape;
}
static NSUInteger OQBroadcastIndex(NSUInteger index, NSArray *outputShape, NSArray *inputShape) {
    NSUInteger result = 0, stride = 1;
    for (NSUInteger i = outputShape.count; i > 0; --i) {
        NSUInteger coordinate = index % [outputShape[i - 1] unsignedIntegerValue];
        index /= [outputShape[i - 1] unsignedIntegerValue];
        if (i > outputShape.count - inputShape.count) {
            NSUInteger d = [inputShape[i - 1 - (outputShape.count - inputShape.count)] unsignedIntegerValue];
            if (d != 1) result += coordinate * stride;
            stride *= d;
        }
    }
    return result;
}
static OQCatalogValue *OQReindex(OQCatalogValue *input, NSArray *shape, NSUInteger (^indexOf)(NSUInteger)) {
    if (!input.data) return nil;
    NSUInteger width = OQWidth(input.dtype), count = OQCount(shape);
    NSMutableData *data = [NSMutableData dataWithLength:OQBytes(shape, input.dtype)];
    for (NSUInteger i = 0; i < count; ++i) {
        NSUInteger from = indexOf(i);
        OQCheck(from < OQCount(input.shape), @"constant index out of bounds");
        memcpy((uint8_t *)data.mutableBytes + i * width, (const uint8_t *)input.data.bytes + from * width, width);
    }
    return OQValue(shape, input.dtype, data, nil);
}
static OQCatalogValue *OQReshaped(MPSGraph *g, OQCatalogValue *input, NSArray *shape, NSString *name) {
    OQCheck(OQCount(input.shape) == OQCount(shape), @"reshape changes element count");
    return OQValue(shape, input.dtype, input.data, input.data ? nil : [g reshapeTensor:OQTensor(g, input) withShape:shape name:name]);
}

static NSArray<NSNumber *> *OQSpatialPads(NSDictionary *a, NSArray *input, NSArray *kernel, NSArray *strides, NSArray *dilations) {
    NSString *mode = OQString(a, @"auto_pad", @"NOTSET");
    NSArray *explicitPads = OQArray(a, @"pads", @[@0,@0,@0,@0]);
    OQCheck(explicitPads.count == 4, @"2D padding requires four values");
    OQCheck([mode isEqual:@"NOTSET"] || a[@"pads"] == nil, @"auto_pad and explicit pads cannot both be specified");
    NSMutableArray *pads = [explicitPads mutableCopy];
    if ([mode isEqual:@"VALID"]) pads = OQFill(4, 0);
    else if ([mode isEqual:@"SAME_UPPER"] || [mode isEqual:@"SAME_LOWER"]) {
        for (NSUInteger i = 0; i < 2; ++i) {
            NSInteger n = [input[i+2] integerValue], s = [strides[i] integerValue];
            NSInteger k = ([kernel[i] integerValue]-1)*[dilations[i] integerValue]+1;
            NSInteger total = MAX(0, ((n+s-1)/s-1)*s+k-n);
            NSInteger first = [mode isEqual:@"SAME_LOWER"] ? (total+1)/2 : total/2;
            pads[i] = @(first); pads[i+2] = @(total-first);
        }
    } else OQCheck([mode isEqual:@"NOTSET"], @"unsupported auto_pad value");
    for (NSNumber *p in pads) OQCheck(p.longLongValue >= 0, @"spatial padding cannot be negative");
    return pads;
}

static NSArray<OQCatalogValue *> *OQTranslate(MPSGraph *g, NSDictionary *node, NSArray *inputs, NSData *blob, NSInteger opset) {
    NSString *op = node[@"op"], *name = node[@"name"];
    NSDictionary *a = node[@"attributes"] ?: @{};
    OQCatalogValue *x = OQInput(inputs, 0, YES), *y = OQInput(inputs, 1, YES);
    OQCatalogValue *r = nil;
    if ([op isEqual:@"Constant"]) {
        OQAttributes(a, @[@"value"]);
        OQCheck(inputs.count == 0, @"Constant must have no inputs");
        r = OQDescriptor(a[@"value"], blob);
    } else if ([op isEqual:@"Shape"]) {
        OQAttributes(a, @[@"start", @"end"]);
        OQCheck(x != nil, @"Shape requires an input");
        NSInteger rank = x.shape.count, start = OQInt(a,@"start",0), end = OQInt(a,@"end",rank);
        if (start < 0) start += rank; if (end < 0) end += rank;
        start = MAX(0,MIN(rank,start)); end = MAX(start,MIN(rank,end));
        r = OQNumbers([x.shape subarrayWithRange:NSMakeRange(start,end-start)], @[@(end-start)], 7);
    } else if ([op isEqual:@"ConstantOfShape"]) {
        OQAttributes(a, @[@"value"]);
        NSArray *shape = OQNumbersOf(x, YES);
        OQCatalogValue *fill = a[@"value"] ? OQDescriptor(a[@"value"],blob) : OQNumbers(@[@0],@[@1],1);
        OQCheck(OQCount(fill.shape) == 1, @"ConstantOfShape value must contain one element");
        NSNumber *value = OQNumber(fill,0);
        NSMutableData *data = [NSMutableData dataWithLength:OQBytes(shape, fill.dtype)];
        for (NSUInteger i=0; i<OQCount(shape); ++i) OQStore(data,fill.dtype,i,value);
        r = OQValue(shape, fill.dtype, data, nil);
    } else if ([op isEqual:@"Add"] || [op isEqual:@"Sub"] || [op isEqual:@"Mul"] || [op isEqual:@"Div"]) {
        OQAttributes(a,@[]);
        OQCheck(x && y && x.dtype == y.dtype, @"binary input dtypes must match");
        NSArray *shape = OQBroadcast(x.shape,y.shape);
        if (x.data && y.data) {
            NSMutableData *data=[NSMutableData dataWithLength:OQBytes(shape,x.dtype)];
            for(NSUInteger i=0;i<OQCount(shape);++i) {
                NSNumber *left=OQNumber(x,OQBroadcastIndex(i,shape,x.shape)), *right=OQNumber(y,OQBroadcastIndex(i,shape,y.shape));
                NSNumber *value;
                if(x.dtype==1) {
                    float l=left.floatValue, q=right.floatValue;
                    value=@([op isEqual:@"Add"]?l+q:[op isEqual:@"Sub"]?l-q:[op isEqual:@"Mul"]?l*q:l/q);
                } else {
                    int64_t l=left.longLongValue, q=right.longLongValue;
                    OQCheck(![op isEqual:@"Div"] || (q && !(l==INT64_MIN && q==-1)), @"invalid constant integer division");
                    value=@([op isEqual:@"Add"]?l+q:[op isEqual:@"Sub"]?l-q:[op isEqual:@"Mul"]?l*q:l/q);
                }
                OQStore(data,x.dtype,i,value);
            }
            r=OQValue(shape,x.dtype,data,nil);
        } else {
            MPSGraphTensor *l=OQTensor(g,x), *q=OQTensor(g,y), *t;
            if([op isEqual:@"Add"]) t=[g additionWithPrimaryTensor:l secondaryTensor:q name:name];
            else if([op isEqual:@"Sub"]) t=[g subtractionWithPrimaryTensor:l secondaryTensor:q name:name];
            else if([op isEqual:@"Mul"]) t=[g multiplicationWithPrimaryTensor:l secondaryTensor:q name:name];
            else t=[g divisionWithPrimaryTensor:l secondaryTensor:q name:name];
            r=OQValue(shape,x.dtype,nil,t);
        }
    } else if ([op isEqual:@"Neg"] || [op isEqual:@"Cast"]) {
        BOOL cast=[op isEqual:@"Cast"];
        OQAttributes(a,cast?@[@"to"]:@[]);
        OQCheck(x!=nil,@"unary operator requires an input");
        NSInteger dtype=cast?OQInt(a,@"to",-1):x.dtype;
        OQType(dtype);
        if(x.data) {
            NSMutableData *data=[NSMutableData dataWithLength:OQBytes(x.shape,dtype)];
            for(NSUInteger i=0;i<OQCount(x.shape);++i) {
                NSNumber *v=OQNumber(x,i);
                if(!cast) v=x.dtype==1?@(-v.floatValue):@(-v.longLongValue);
                OQStore(data,dtype,i,v);
            }
            r=OQValue(x.shape,dtype,data,nil);
        } else r=OQValue(x.shape,dtype,nil,cast?[g castTensor:OQTensor(g,x) toType:OQType(dtype) name:name]:[g negativeWithTensor:OQTensor(g,x) name:name]);
    } else if ([op isEqual:@"Reshape"] || [op isEqual:@"Unsqueeze"] || [op isEqual:@"Squeeze"]) {
        OQCheck(x!=nil,@"shape operator requires an input");
        NSMutableArray *shape;
        if([op isEqual:@"Reshape"]) {
            OQAttributes(a,@[@"allowzero"]);
            NSArray *requested=OQNumbersOf(y,YES);
            OQCheck(y.shape.count==1,@"Reshape shape input must be a vector");
            shape=[requested mutableCopy]; NSInteger infer=-1; NSUInteger known=1;
            BOOL allowzero=OQInt(a,@"allowzero",0)!=0;
            for(NSUInteger i=0;i<shape.count;++i) {
                NSInteger d=[shape[i] integerValue];
                if(d==0 && !allowzero) { OQCheck(i<x.shape.count,@"Reshape zero dimension has no source dimension"); shape[i]=x.shape[i]; d=[shape[i] integerValue]; }
                if(d==-1) { OQCheck(infer==-1,@"Reshape has multiple inferred dimensions"); infer=i; }
                else { OQCheck(d>=0 && (!d || known<=NSUIntegerMax/(NSUInteger)d),@"invalid Reshape dimension"); known*=d; }
            }
            if(infer>=0) { OQCheck(known>0 && OQCount(x.shape)%known==0,@"Reshape inferred dimension is not integral"); shape[infer]=@(OQCount(x.shape)/known); }
        } else {
            OQAttributes(a,@[@"axes"]);
            OQCheck(!(a[@"axes"] && y),@"axes cannot be supplied both as attribute and input");
            NSArray *axes=y?OQNumbersOf(y,YES):a[@"axes"];
            if([op isEqual:@"Unsqueeze"]) {
                OQCheck(axes!=nil,@"Unsqueeze requires axes");
                NSUInteger rank=x.shape.count+axes.count;
                NSMutableIndexSet *positions=[NSMutableIndexSet indexSet];
                for(NSNumber *axis in axes) { NSInteger p=OQAxis(axis.integerValue,rank); OQCheck(![positions containsIndex:p],@"duplicate Unsqueeze axis"); [positions addIndex:p]; }
                shape=OQFill(rank,1); NSUInteger from=0;
                for(NSUInteger i=0;i<rank;++i) if(![positions containsIndex:i]) shape[i]=x.shape[from++];
            } else {
                NSMutableIndexSet *positions=[NSMutableIndexSet indexSet];
                if(axes) for(NSNumber *axis in axes) { NSInteger p=OQAxis(axis.integerValue,x.shape.count); OQCheck([x.shape[p] integerValue]==1,@"Squeeze dimension must be one"); OQCheck(![positions containsIndex:p],@"duplicate Squeeze axis"); [positions addIndex:p]; }
                else for(NSUInteger i=0;i<x.shape.count;++i) if([x.shape[i] integerValue]==1) [positions addIndex:i];
                shape=[x.shape mutableCopy]; [shape removeObjectsAtIndexes:positions];
            }
        }
        r=OQReshaped(g,x,shape,name);
    } else if ([op isEqual:@"Transpose"]) {
        OQAttributes(a,@[@"perm"]); OQCheck(x!=nil,@"Transpose requires an input");
        NSArray *perm=a[@"perm"];
        if(!perm) { NSMutableArray *reverse=[NSMutableArray array]; for(NSUInteger i=x.shape.count;i>0;--i) [reverse addObject:@(i-1)]; perm=reverse; }
        OQCheck(perm.count==x.shape.count,@"Transpose permutation rank mismatch");
        NSMutableIndexSet *seen=[NSMutableIndexSet indexSet]; NSMutableArray *shape=[NSMutableArray array];
        for(NSNumber *p in perm) { NSInteger axis=OQAxis(p.integerValue,x.shape.count); OQCheck(p.integerValue>=0 && ![seen containsIndex:axis],@"invalid Transpose permutation"); [seen addIndex:axis]; [shape addObject:x.shape[axis]]; }
        r=OQReindex(x,shape,^NSUInteger(NSUInteger index) {
            NSUInteger coords[16]={0}, from=0, stride=1;
            for(NSUInteger i=shape.count;i>0;--i) { coords[[perm[i-1] unsignedIntegerValue]]=index%[shape[i-1] unsignedIntegerValue]; index/=[shape[i-1] unsignedIntegerValue]; }
            for(NSUInteger i=x.shape.count;i>0;--i) { from+=coords[i-1]*stride; stride*=[x.shape[i-1] unsignedIntegerValue]; }
            return from;
        });
        if(!r) r=OQValue(shape,x.dtype,nil,[g transposeTensor:OQTensor(g,x) permutation:perm name:name]);
    } else if ([op isEqual:@"Gather"]) {
        OQAttributes(a,@[@"axis"]); OQCheck(x && y,@"Gather requires two inputs");
        NSInteger axis=OQAxis(OQInt(a,@"axis",0),x.shape.count);
        NSArray *indices=OQNumbersOf(y,YES); NSMutableArray *normalized=[NSMutableArray array];
        NSInteger dimension=[x.shape[axis] integerValue];
        for(NSNumber *n in indices) { int64_t v=n.longLongValue; if(v<0) v+=dimension; OQCheck(v>=0 && v<dimension,@"Gather index out of range"); [normalized addObject:@(v)]; }
        OQCatalogValue *indexValue=OQNumbers(normalized,y.shape,y.dtype);
        NSMutableArray *shape=[x.shape mutableCopy]; [shape removeObjectAtIndex:axis]; [shape insertObjects:y.shape atIndexes:[NSIndexSet indexSetWithIndexesInRange:NSMakeRange(axis,y.shape.count)]];
        NSUInteger inner=1; for(NSUInteger i=axis+1;i<x.shape.count;++i) inner*=[x.shape[i] unsignedIntegerValue];
        NSUInteger indexCount=indices.count;
        r=OQReindex(x,shape,^NSUInteger(NSUInteger index) { NSUInteger outer=index/(indexCount*inner), j=(index/inner)%indexCount; return (outer*dimension+[normalized[j] unsignedIntegerValue])*inner+index%inner; });
        if(!r) r=OQValue(shape,x.dtype,nil,[g gatherWithUpdatesTensor:OQTensor(g,x) indicesTensor:OQTensor(g,indexValue) axis:axis batchDimensions:0 name:name]);
    } else if ([op isEqual:@"Slice"]) {
        OQAttributes(a,@[@"starts",@"ends",@"axes",@"steps"]); OQCheck(x!=nil,@"Slice requires an input");
        NSArray *starts=y?OQNumbersOf(y,YES):a[@"starts"], *ends=OQInput(inputs,2,YES)?OQNumbersOf(OQInput(inputs,2,YES),YES):a[@"ends"];
        NSArray *axes=OQInput(inputs,3,YES)?OQNumbersOf(OQInput(inputs,3,YES),YES):a[@"axes"], *steps=OQInput(inputs,4,YES)?OQNumbersOf(OQInput(inputs,4,YES),YES):a[@"steps"];
        OQCheck(starts && ends && starts.count==ends.count && (!axes || axes.count==starts.count) && (!steps || steps.count==starts.count),@"invalid Slice parameters");
        NSMutableArray *begin=OQFill(x.shape.count,0), *end=[x.shape mutableCopy], *stride=OQFill(x.shape.count,1), *shape=[x.shape mutableCopy];
        NSMutableIndexSet *seen=[NSMutableIndexSet indexSet]; uint32_t endMask=0;
        for(NSUInteger i=0;i<starts.count;++i) {
            NSInteger axis=OQAxis(axes?[axes[i] integerValue]:i,x.shape.count); OQCheck(![seen containsIndex:axis],@"duplicate Slice axis"); [seen addIndex:axis];
            int64_t n=[x.shape[axis] longLongValue], s=[starts[i] longLongValue], e=[ends[i] longLongValue], step=steps?[steps[i] longLongValue]:1;
            OQCheck(step!=0 && step!=INT64_MIN,@"invalid Slice step");
            if(s<0) s+=n; if(e<0 && e!=INT64_MIN) e+=n;
            s=MAX(step>0?0:-1,MIN(step>0?n:n-1,s)); e=MAX(step>0?0:-1,MIN(step>0?n:n-1,e));
            int64_t count=step>0?(e>s?(e-s-1)/step+1:0):(s>e?(s-e-1)/(-step)+1:0);
            begin[axis]=@(s); end[axis]=@(e); stride[axis]=@(step); shape[axis]=@(count);
            if(step<0 && e==-1) endMask|=1u<<axis;
        }
        r=OQReindex(x,shape,^NSUInteger(NSUInteger index) {
            NSUInteger from=0, mul=1;
            for(NSUInteger i=shape.count;i>0;--i) { NSInteger coord=index%[shape[i-1] unsignedIntegerValue]; index/=[shape[i-1] unsignedIntegerValue]; from+=([begin[i-1] integerValue]+coord*[stride[i-1] integerValue])*mul; mul*=[x.shape[i-1] unsignedIntegerValue]; }
            return from;
        });
        if(!r) { OQCheck(OQCount(shape)>0,@"empty dynamic Slice is unsupported"); r=OQValue(shape,x.dtype,nil,[g sliceTensor:OQTensor(g,x) starts:begin ends:end strides:stride startMask:0 endMask:endMask squeezeMask:0 name:name]); }
    } else if ([op isEqual:@"Concat"]) {
        OQAttributes(a,@[@"axis"]); OQCheck(x && inputs.count>0,@"Concat requires inputs");
        NSInteger axis=OQAxis(OQInt(a,@"axis",0),x.shape.count); NSMutableArray *shape=[x.shape mutableCopy]; shape[axis]=@0;
        BOOL constant=YES; NSMutableArray *tensors=[NSMutableArray array];
        for(OQCatalogValue *v in inputs) {
            OQCheck(v!= (id)NSNull.null && v.dtype==x.dtype && v.shape.count==shape.count,@"incompatible Concat input");
            for(NSUInteger i=0;i<shape.count;++i) if(i!=(NSUInteger)axis) OQCheck([v.shape[i] isEqual:shape[i]],@"Concat non-axis dimensions must match");
            shape[axis]=@([shape[axis] unsignedIntegerValue]+[v.shape[axis] unsignedIntegerValue]); constant&=v.data!=nil;
        }
        if(constant) {
            NSUInteger inner=1, outer=1, width=OQWidth(x.dtype);
            for(NSUInteger i=axis+1;i<x.shape.count;++i) inner*=[x.shape[i] unsignedIntegerValue];
            for(NSUInteger i=0;i<(NSUInteger)axis;++i) outer*=[x.shape[i] unsignedIntegerValue];
            NSMutableData *data=[NSMutableData dataWithLength:OQBytes(shape,x.dtype)]; NSUInteger at=0;
            for(NSUInteger o=0;o<outer;++o) for(OQCatalogValue *v in inputs) { NSUInteger block=[v.shape[axis] unsignedIntegerValue]*inner*width; if(block) memcpy((uint8_t *)data.mutableBytes+at,(const uint8_t *)v.data.bytes+o*block,block); at+=block; }
            r=OQValue(shape,x.dtype,data,nil);
        } else {
            for(OQCatalogValue *v in inputs) if(OQCount(v.shape)) [tensors addObject:OQTensor(g,v)];
            OQCheck(tensors.count>0,@"Concat has no nonempty inputs");
            r=OQValue(shape,x.dtype,nil,[g concatTensors:tensors dimension:axis name:name]);
        }
    } else if ([op isEqual:@"Expand"]) {
        OQAttributes(a,@[]); OQCheck(x && y,@"Expand requires two inputs");
        NSArray *shape=OQBroadcast(x.shape,OQNumbersOf(y,YES));
        r=OQReindex(x,shape,^NSUInteger(NSUInteger index) { return OQBroadcastIndex(index,shape,x.shape); });
        if(!r) r=OQValue(shape,x.dtype,nil,[g broadcastTensor:OQTensor(g,x) toShape:shape name:name]);
    } else if ([op isEqual:@"Conv"] || [op isEqual:@"MaxPool"]) {
        BOOL conv=[op isEqual:@"Conv"];
        OQAttributes(a,conv?@[@"auto_pad",@"dilations",@"group",@"kernel_shape",@"pads",@"strides"]:@[@"auto_pad",@"ceil_mode",@"dilations",@"kernel_shape",@"pads",@"storage_order",@"strides"]);
        OQCheck(x && x.dtype==1 && x.shape.count==4,@"Conv/MaxPool requires NCHW FP32 rank four");
        if(conv) OQCheck(y && y.dtype==1 && y.shape.count==4,@"Conv weights require OIHW FP32 rank four");
        NSArray *kernel=OQArray(a,@"kernel_shape",conv?[y.shape subarrayWithRange:NSMakeRange(2,2)]:nil), *strides=OQArray(a,@"strides",@[@1,@1]), *dilations=OQArray(a,@"dilations",@[@1,@1]);
        OQCheck(kernel.count==2 && strides.count==2 && dilations.count==2,@"Conv/MaxPool spatial parameter rank mismatch");
        for(NSUInteger i=0;i<2;++i) OQCheck([kernel[i] integerValue]>0 && [strides[i] integerValue]>0 && [dilations[i] integerValue]>0,@"Conv/MaxPool parameters must be positive");
        NSArray *pads=OQSpatialPads(a,x.shape,kernel,strides,dilations); NSMutableArray *shape=[x.shape mutableCopy];
        BOOL ceilMode=!conv && OQInt(a,@"ceil_mode",0)!=0;
        for(NSUInteger i=0;i<2;++i) {
            NSInteger n=[x.shape[i+2] integerValue], stride=[strides[i] integerValue], effective=([kernel[i] integerValue]-1)*[dilations[i] integerValue]+1;
            double numerator=(double)n+[pads[i] integerValue]+[pads[i+2] integerValue]-effective;
            NSInteger out=(NSInteger)(ceilMode?ceil(numerator/stride):floor(numerator/stride))+1;
            if(ceilMode && (out-1)*stride>=n+[pads[i] integerValue]) --out;
            OQCheck(out>0,@"Conv/MaxPool produced a nonpositive spatial dimension"); shape[i+2]=@(out);
        }
        MPSGraphTensor *t;
        if(conv) {
            NSInteger groups=OQInt(a,@"group",1), in=[x.shape[1] integerValue], out=[y.shape[0] integerValue];
            OQCheck(groups>0 && in%groups==0 && out%groups==0 && [y.shape[1] integerValue]==in/groups,@"invalid grouped convolution channels");
            OQCheck([kernel[0] isEqual:y.shape[2]] && [kernel[1] isEqual:y.shape[3]],@"Conv kernel_shape disagrees with weights"); shape[1]=@(out);
            MPSGraphConvolution2DOpDescriptor *d=[MPSGraphConvolution2DOpDescriptor descriptorWithStrideInX:[strides[1] unsignedIntegerValue] strideInY:[strides[0] unsignedIntegerValue] dilationRateInX:[dilations[1] unsignedIntegerValue] dilationRateInY:[dilations[0] unsignedIntegerValue] groups:groups paddingLeft:[pads[1] unsignedIntegerValue] paddingRight:[pads[3] unsignedIntegerValue] paddingTop:[pads[0] unsignedIntegerValue] paddingBottom:[pads[2] unsignedIntegerValue] paddingStyle:MPSGraphPaddingStyleExplicit dataLayout:MPSGraphTensorNamedDataLayoutNCHW weightsLayout:MPSGraphTensorNamedDataLayoutOIHW];
            OQCheck(d!=nil,@"Conv descriptor allocation failed");
            t=[g convolution2DWithSourceTensor:OQTensor(g,x) weightsTensor:OQTensor(g,y) descriptor:d name:name];
            OQCatalogValue *bias=OQInput(inputs,2,YES);
            if(bias) { OQCheck(bias.dtype==1 && [bias.shape isEqual:@[@(out)]],@"Conv bias shape mismatch"); MPSGraphTensor *b=[g reshapeTensor:OQTensor(g,bias) withShape:@[@1,@(out),@1,@1] name:nil]; t=[g additionWithPrimaryTensor:t secondaryTensor:b name:nil]; }
        } else {
            OQCheck(OQInt(a,@"storage_order",0)==0 && [node[@"outputs"] count]==1,@"MaxPool indices/storage_order are unsupported for catalog graphs");
            MPSGraphPooling2DOpDescriptor *d=[MPSGraphPooling2DOpDescriptor descriptorWithKernelWidth:[kernel[1] unsignedIntegerValue] kernelHeight:[kernel[0] unsignedIntegerValue] strideInX:[strides[1] unsignedIntegerValue] strideInY:[strides[0] unsignedIntegerValue] dilationRateInX:[dilations[1] unsignedIntegerValue] dilationRateInY:[dilations[0] unsignedIntegerValue] paddingLeft:[pads[1] unsignedIntegerValue] paddingRight:[pads[3] unsignedIntegerValue] paddingTop:[pads[0] unsignedIntegerValue] paddingBottom:[pads[2] unsignedIntegerValue] paddingStyle:MPSGraphPaddingStyleExplicit dataLayout:MPSGraphTensorNamedDataLayoutNCHW];
            OQCheck(d!=nil,@"MaxPool descriptor allocation failed"); d.ceilMode=ceilMode;
            t=[g maxPooling2DWithSourceTensor:OQTensor(g,x) descriptor:d name:name];
        }
        r=OQValue(shape,1,nil,t);
    } else if ([op isEqual:@"Relu"] || [op isEqual:@"Sigmoid"] || [op isEqual:@"PRelu"] || [op isEqual:@"Clip"]) {
        OQAttributes(a,[op isEqual:@"Clip"]?@[@"min",@"max"]:@[]); OQCheck(x && x.dtype==1,@"activation requires FP32 input");
        MPSGraphTensor *t=OQTensor(g,x);
        if([op isEqual:@"Relu"]) t=[g reLUWithTensor:t name:name];
        else if([op isEqual:@"Sigmoid"]) t=[g sigmoidWithTensor:t name:name];
        else if([op isEqual:@"PRelu"]) {
            OQCheck(y && y.dtype==1 && [OQBroadcast(x.shape,y.shape) isEqual:x.shape],@"PRelu slope broadcast mismatch");
            MPSGraphTensor *zero=[g constantWithScalar:0 dataType:MPSDataTypeFloat32];
            MPSGraphTensor *positive=[g maximumWithPrimaryTensor:t secondaryTensor:zero name:nil], *negative=[g minimumWithPrimaryTensor:t secondaryTensor:zero name:nil];
            t=[g additionWithPrimaryTensor:positive secondaryTensor:[g multiplicationWithPrimaryTensor:negative secondaryTensor:OQTensor(g,y) name:nil] name:name];
        } else {
            OQCatalogValue *upper=OQInput(inputs,2,YES);
            OQCheck((!y || (y.dtype==1 && OQCount(y.shape)==1)) && (!upper || (upper.dtype==1 && OQCount(upper.shape)==1)),@"Clip bounds must be scalar FP32");
            MPSGraphTensor *lo=y?OQTensor(g,y):[g constantWithScalar:OQFloat(a,@"min",-FLT_MAX) dataType:MPSDataTypeFloat32];
            MPSGraphTensor *hi=upper?OQTensor(g,upper):[g constantWithScalar:OQFloat(a,@"max",FLT_MAX) dataType:MPSDataTypeFloat32];
            t=[g clampWithTensor:t minValueTensor:lo maxValueTensor:hi name:name];
        }
        r=OQValue(x.shape,1,nil,t);
    } else if ([op isEqual:@"DepthToSpace"]) {
        OQAttributes(a,@[@"blocksize",@"mode"]); OQCheck(x && x.shape.count==4,@"DepthToSpace requires NCHW rank four");
        NSInteger b=OQInt(a,@"blocksize",0), c=[x.shape[1] integerValue], h=[x.shape[2] integerValue], w=[x.shape[3] integerValue];
        OQCheck(b>0 && b<=INT_MAX && c%(b*b)==0,@"DepthToSpace blocksize/channel mismatch"); c/=b*b;
        NSString *mode=OQString(a,@"mode",@"DCR"); OQCheck([mode isEqual:@"DCR"] || [mode isEqual:@"CRD"],@"unsupported DepthToSpace mode");
        // Explicit reshape/transpose distinguishes ONNX DCR from PyTorch CRD.
        NSArray *intermediate=[mode isEqual:@"DCR"]?@[x.shape[0],@(b),@(b),@(c),@(h),@(w)]:@[x.shape[0],@(c),@(b),@(b),@(h),@(w)];
        NSArray *perm=[mode isEqual:@"DCR"]?@[@0,@3,@4,@1,@5,@2]:@[@0,@1,@4,@2,@5,@3];
        NSArray *shape=@[x.shape[0],@(c),@(h*b),@(w*b)];
        MPSGraphTensor *t=[g reshapeTensor:OQTensor(g,x) withShape:intermediate name:nil]; t=[g transposeTensor:t permutation:perm name:nil]; t=[g reshapeTensor:t withShape:shape name:name];
        r=OQValue(shape,x.dtype,nil,t);
    } else if ([op isEqual:@"Resize"]) {
        OQAttributes(a,@[@"coordinate_transformation_mode",@"cubic_coeff_a",@"exclude_outside",@"extrapolation_value",@"mode",@"nearest_mode",@"antialias"]);
        OQCheck(x && x.dtype==1 && x.shape.count==4,@"Resize requires NCHW FP32 rank four");
        OQCheck(OQInt(a,@"exclude_outside",0)==0 && OQInt(a,@"antialias",0)==0 && OQFloat(a,@"extrapolation_value",0)==0,@"unsupported Resize extrapolation/exclusion/antialias attributes");
        OQCheck(OQFloat(a,@"cubic_coeff_a",-0.75)==-0.75,@"unsupported Resize cubic coefficient");
        OQCatalogValue *roi=OQInput(inputs,1,YES), *scales=OQInput(inputs,2,YES), *sizes=OQInput(inputs,3,YES);
        OQCheck(!roi || OQCount(roi.shape)==0,@"Resize ROI is only supported for crop-and-resize, absent from catalog");
        BOOL haveScales=scales && OQCount(scales.shape), haveSizes=sizes && OQCount(sizes.shape);
        OQCheck(haveScales!=haveSizes,@"Resize requires exactly one nonempty scales/sizes input");
        NSMutableArray *shape;
        if(haveSizes) { shape=[OQNumbersOf(sizes,YES) mutableCopy]; OQCheck(shape.count==4,@"Resize sizes must contain four dimensions"); }
        else {
            NSArray *factors=OQNumbersOf(scales,NO); OQCheck(factors.count==4 && scales.dtype==1,@"Resize scales must be FP32 vector length four"); shape=[NSMutableArray array];
            for(NSUInteger i=0;i<4;++i) { double scale=[factors[i] doubleValue]; OQCheck(isfinite(scale) && scale>0,@"Resize scale must be positive and finite"); [shape addObject:@((NSInteger)floor([x.shape[i] doubleValue]*scale))]; }
        }
        OQCheck([shape[0] isEqual:x.shape[0]] && [shape[1] isEqual:x.shape[1]] && [shape[2] integerValue]>0 && [shape[3] integerValue]>0,@"Resize may only change positive spatial dimensions");
        NSString *mode=OQString(a,@"mode",@"nearest"), *coordinate=OQString(a,@"coordinate_transformation_mode",@"half_pixel"), *nearest=OQString(a,@"nearest_mode",@"round_prefer_floor");
        BOOL linear=[mode isEqual:@"linear"], align=[coordinate isEqual:@"align_corners"], half=[coordinate isEqual:@"half_pixel"] || [coordinate isEqual:@"pytorch_half_pixel"];
        OQCheck(linear || [mode isEqual:@"nearest"],@"unsupported Resize interpolation mode");
        OQCheck(align || half || [coordinate isEqual:@"asymmetric"],@"unsupported Resize coordinate transformation");
        // The catalog's scale tensors all express integral spatial scaling. MPSGraph
        // derives its transform from sizes, so reject other scale ratios rather than
        // silently changing ONNX's original scale-based coordinate transform.
        if(haveScales) {
            NSArray *factors=OQNumbersOf(scales,NO);
            for(NSUInteger i=0;i<4;++i) OQCheck([shape[i] doubleValue]/[x.shape[i] doubleValue]==[factors[i] doubleValue],@"Resize scales do not exactly match prepared output size");
        }
        MPSGraphTensor *t;
        NSArray *spatial=@[shape[2],shape[3]];
        if(linear) {
            OQCheck([nearest isEqual:@"floor"] || [nearest isEqual:@"round_prefer_floor"],@"unsupported inactive Resize nearest_mode");
            // pytorch_half_pixel uses source coordinate zero when output_length=1.
            OQCatalogValue *source=x;
            if([coordinate isEqual:@"pytorch_half_pixel"]) {
                for(NSUInteger axis=2;axis<4;++axis) if([shape[axis] integerValue]==1 && [source.shape[axis] integerValue]!=1) {
                    NSMutableArray *cropped=[source.shape mutableCopy]; cropped[axis]=@1;
                    source=OQValue(cropped,1,nil,[g sliceTensor:OQTensor(g,source) dimension:axis start:0 length:1 name:nil]);
                }
            }
            t=[g resizeTensor:OQTensor(g,source) size:spatial mode:MPSGraphResizeBilinear centerResult:half alignCorners:align layout:MPSGraphTensorNamedDataLayoutNCHW name:name];
        } else {
            OQCheck([coordinate isEqual:@"asymmetric"] && [nearest isEqual:@"floor"],@"catalog nearest Resize requires asymmetric/floor");
            OQCatalogValue *size=OQNumbers(spatial,@[@2],6);
            t=[g resizeNearestWithTensor:OQTensor(g,x) sizeTensor:OQTensor(g,size) nearestRoundingMode:MPSGraphResizeNearestRoundingModeFloor centerResult:NO alignCorners:NO layout:MPSGraphTensorNamedDataLayoutNCHW name:name];
        }
        r=OQValue(shape,1,nil,t);
    } else if ([op isEqual:@"BatchNormalization"] || [op isEqual:@"InstanceNormalization"]) {
        BOOL batch=[op isEqual:@"BatchNormalization"];
        OQAttributes(a,batch?@[@"epsilon",@"momentum",@"training_mode"]:@[@"epsilon"]);
        OQCheck(x && x.dtype==1 && x.shape.count>=3,@"normalization requires FP32 rank at least three");
        OQCheck(!batch || (OQInt(a,@"training_mode",0)==0 && [node[@"outputs"] count]==1),@"BatchNormalization must use inference mode and one output");
        double epsilon=OQFloat(a,@"epsilon",1e-5); OQCheck(isfinite(epsilon) && epsilon>=0,@"invalid normalization epsilon");
        if(batch) { double momentum=OQFloat(a,@"momentum",0.9); OQCheck(isfinite(momentum) && momentum>=0 && momentum<=1,@"invalid BatchNormalization momentum"); }
        NSMutableArray *channelShape=OQFill(x.shape.count,1); channelShape[1]=x.shape[1];
        OQCatalogValue *bias=OQInput(inputs,2,NO);
        OQCheck(y && y.dtype==1 && bias.dtype==1 && [y.shape isEqual:@[x.shape[1]]] && [bias.shape isEqual:y.shape],@"normalization scale/bias shape mismatch");
        MPSGraphTensor *gamma=[g reshapeTensor:OQTensor(g,y) withShape:channelShape name:nil], *beta=[g reshapeTensor:OQTensor(g,bias) withShape:channelShape name:nil], *mean, *variance;
        if(batch) {
            OQCatalogValue *m=OQInput(inputs,3,NO), *v=OQInput(inputs,4,NO);
            OQCheck(m.dtype==1 && v.dtype==1 && [m.shape isEqual:y.shape] && [v.shape isEqual:y.shape],@"BatchNormalization mean/variance shape mismatch");
            mean=[g reshapeTensor:OQTensor(g,m) withShape:channelShape name:nil]; variance=[g reshapeTensor:OQTensor(g,v) withShape:channelShape name:nil];
        } else {
            NSMutableArray *axes=[NSMutableArray array]; for(NSUInteger i=2;i<x.shape.count;++i) [axes addObject:@(i)];
            mean=[g meanOfTensor:OQTensor(g,x) axes:axes name:nil]; variance=[g varianceOfTensor:OQTensor(g,x) meanTensor:mean axes:axes name:nil];
        }
        r=OQValue(x.shape,1,nil,[g normalizationWithTensor:OQTensor(g,x) meanTensor:mean varianceTensor:variance gammaTensor:gamma betaTensor:beta epsilon:(float)epsilon name:name]);
    } else if ([op isEqual:@"GlobalAveragePool"]) {
        OQAttributes(a,@[]); OQCheck(x && x.dtype==1 && x.shape.count>=3,@"GlobalAveragePool requires FP32 spatial tensor");
        NSMutableArray *axes=[NSMutableArray array], *shape=[x.shape mutableCopy];
        for(NSUInteger i=2;i<x.shape.count;++i) { [axes addObject:@(i)]; shape[i]=@1; }
        r=OQValue(shape,1,nil,[g meanOfTensor:OQTensor(g,x) axes:axes name:name]);
    } else if ([op isEqual:@"MatMul"]) {
        OQAttributes(a,@[]); OQCheck(x && y && x.dtype==1 && y.dtype==1 && x.shape.count>=1 && y.shape.count>=1,@"MatMul requires non-scalar FP32 tensors");
        BOOL leftVector=x.shape.count==1, rightVector=y.shape.count==1;
        NSMutableArray *left=[x.shape mutableCopy], *right=[y.shape mutableCopy];
        if(leftVector) [left insertObject:@1 atIndex:0]; if(rightVector) [right addObject:@1];
        OQCheck([left.lastObject isEqual:right[right.count-2]],@"MatMul contracting dimensions mismatch");
        NSArray *batch=OQBroadcast([left subarrayWithRange:NSMakeRange(0,left.count-2)],[right subarrayWithRange:NSMakeRange(0,right.count-2)]);
        NSMutableArray *shape=[batch mutableCopy]; [shape addObject:left[left.count-2]]; [shape addObject:right.lastObject];
        MPSGraphTensor *l=OQTensor(g,x), *q=OQTensor(g,y);
        if(leftVector) l=[g reshapeTensor:l withShape:left name:nil]; if(rightVector) q=[g reshapeTensor:q withShape:right name:nil];
        MPSGraphTensor *t=[g matrixMultiplicationWithPrimaryTensor:l secondaryTensor:q name:name];
        if(rightVector) [shape removeLastObject]; if(leftVector) [shape removeObjectAtIndex:batch.count];
        if(leftVector || rightVector) t=[g reshapeTensor:t withShape:shape name:nil];
        r=OQValue(shape,1,nil,t);
    } else if ([op isEqual:@"Softmax"]) {
        OQAttributes(a,@[@"axis"]); OQCheck(x && x.dtype==1,@"Softmax requires FP32 tensor");
        NSInteger axis=OQAxis(OQInt(a,@"axis",opset>=13?-1:1),x.shape.count); MPSGraphTensor *t=OQTensor(g,x);
        if(opset<13) {
            NSUInteger outer=1,inner=1; for(NSUInteger i=0;i<x.shape.count;++i) { if(i<(NSUInteger)axis) outer*=[x.shape[i] unsignedIntegerValue]; else inner*=[x.shape[i] unsignedIntegerValue]; }
            t=[g reshapeTensor:t withShape:@[@(outer),@(inner)] name:nil]; t=[g softMaxWithTensor:t axis:1 name:name]; t=[g reshapeTensor:t withShape:x.shape name:nil];
        } else t=[g softMaxWithTensor:t axis:axis name:name];
        r=OQValue(x.shape,1,nil,t);
    } else if ([op isEqual:@"ArgMax"]) {
        OQAttributes(a,@[@"axis",@"keepdims",@"select_last_index"]); OQCheck(x && x.dtype==1,@"ArgMax requires FP32 tensor");
        OQCheck(OQInt(a,@"select_last_index",0)==0,@"ArgMax requires first-index tie selection");
        NSInteger axis=OQAxis(OQInt(a,@"axis",0),x.shape.count); NSUInteger dimension=[x.shape[axis] unsignedIntegerValue];
        OQCheck(dimension>0 && dimension<=INT32_MAX,@"ArgMax dimension exceeds index range");
        NSMutableArray *indices=[NSMutableArray arrayWithCapacity:dimension]; for(NSUInteger i=0;i<dimension;++i) [indices addObject:@(i)];
        NSMutableArray *indexShape=OQFill(x.shape.count,1); indexShape[axis]=@(dimension);
        OQCatalogValue *iv=OQNumbers(indices,indexShape,6);
        MPSGraphTensor *maximum=[g reductionMaximumWithTensor:OQTensor(g,x) axis:axis name:nil];
        MPSGraphTensor *equal=[g equalWithPrimaryTensor:OQTensor(g,x) secondaryTensor:maximum name:nil];
        MPSGraphTensor *sentinel=[g constantWithScalar:(double)INT32_MAX dataType:MPSDataTypeInt32];
        MPSGraphTensor *matching=[g selectWithPredicateTensor:equal truePredicateTensor:OQTensor(g,iv) falsePredicateTensor:sentinel name:nil];
        MPSGraphTensor *t=[g reductionMinimumWithTensor:matching axis:axis name:nil];
        NSMutableArray *shape=[x.shape mutableCopy]; shape[axis]=@1;
        if(!OQInt(a,@"keepdims",1)) { [shape removeObjectAtIndex:axis]; t=[g reshapeTensor:t withShape:shape name:nil]; }
        t=[g castTensor:t toType:MPSDataTypeInt64 name:name];
        r=OQValue(shape,7,nil,t);
    } else if ([op isEqual:@"Split"]) {
        OQAttributes(a,@[@"axis",@"split",@"num_outputs"]); OQCheck(x!=nil,@"Split requires an input");
        NSInteger axis=OQAxis(OQInt(a,@"axis",0),x.shape.count); NSUInteger outputCount=[node[@"outputs"] count];
        OQCheck(outputCount>0 && !(a[@"split"] && y),@"invalid Split output count or duplicate sizes");
        NSArray *sizes=y?OQNumbersOf(y,YES):a[@"split"];
        if(!sizes) { OQCheck(OQInt(a,@"num_outputs",outputCount)==(NSInteger)outputCount && [x.shape[axis] unsignedIntegerValue]%outputCount==0,@"catalog Split requires equal divisible partitions"); sizes=OQFill(outputCount,[x.shape[axis] unsignedIntegerValue]/outputCount); }
        OQCheck(sizes.count==outputCount,@"Split sizes/output count mismatch"); NSUInteger sum=0; for(NSNumber *n in sizes) { OQCheck(n.longLongValue>0,@"Split sizes must be positive"); sum+=n.unsignedIntegerValue; }
        OQCheck(sum==[x.shape[axis] unsignedIntegerValue],@"Split sizes do not cover axis");
        NSArray *tensors=[g splitTensor:OQTensor(g,x) splitSizes:sizes axis:axis name:name]; OQCheck(tensors.count==outputCount,@"MPSGraph Split output count mismatch");
        NSMutableArray *results=[NSMutableArray array]; for(NSUInteger i=0;i<outputCount;++i) { NSMutableArray *shape=[x.shape mutableCopy]; shape[axis]=sizes[i]; [results addObject:OQValue(shape,x.dtype,nil,tensors[i])]; }
        return results;
    } else if ([op isEqual:@"Pad"]) {
        OQAttributes(a,@[@"mode",@"pads",@"value"]); OQCheck(x && x.dtype==1,@"Pad requires FP32 input");
        OQCheck([OQString(a,@"mode",@"constant") isEqual:@"constant"],@"catalog Pad only supports constant mode");
        NSArray *pads=y?OQNumbersOf(y,YES):a[@"pads"]; OQCheck(pads.count==x.shape.count*2,@"Pad parameters rank mismatch");
        OQCatalogValue *fill=OQInput(inputs,2,YES); OQCheck(!fill || (fill.dtype==1 && OQCount(fill.shape)==1),@"Pad constant value must be scalar FP32");
        double value=fill?OQNumber(fill,0).doubleValue:OQFloat(a,@"value",0);
        NSMutableArray *before=OQFill(x.shape.count,0), *after=OQFill(x.shape.count,0), *shape=[x.shape mutableCopy];
        MPSGraphTensor *t=OQTensor(g,x);
        for(NSUInteger i=0;i<x.shape.count;++i) {
            NSInteger left=[pads[i] integerValue],right=[pads[i+x.shape.count] integerValue],n=[x.shape[i] integerValue];
            NSInteger start=MAX(0,-left), length=n-start-MAX(0,-right); OQCheck(length>0,@"Pad cropping removes an entire dimension");
            if(left<0 || right<0) t=[g sliceTensor:t dimension:i start:start length:length name:nil];
            before[i]=@(MAX(0,left)); after[i]=@(MAX(0,right)); shape[i]=@(n+left+right); OQCheck([shape[i] integerValue]>0,@"Pad result dimension must be positive");
        }
        t=[g padTensor:t withPaddingMode:MPSGraphPaddingModeConstant leftPadding:before rightPadding:after constantValue:value name:name];
        r=OQValue(shape,1,nil,t);
    } else OQFail(@"unsupported ONNX operator %@",op);
    OQCheck(r!=nil,@"operator produced no result");
    return @[r];
}

@interface OQCatalogGraph : NSObject
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, strong) MPSGraph *graph;
@property(nonatomic, strong) MPSGraphExecutable *executable;
@property(nonatomic, copy) NSArray *inputShape;
@property(nonatomic, copy) NSArray *outputShape;
@property(nonatomic) NSUInteger inputBytes;
@property(nonatomic) NSUInteger outputBytes;
@property(nonatomic, strong) NSMutableDictionary<NSValue *, NSArray<MPSGraphTensorData *> *> *inputCache;
@property(nonatomic, strong) NSMutableDictionary<NSValue *, NSArray<MPSGraphTensorData *> *> *outputCache;
@property(nonatomic, copy) NSString *asynchronousError;
@end
@implementation OQCatalogGraph
@end

static void OQRecordError(OQCatalogGraph *owner, NSString *message) {
    @synchronized(owner) { if(!owner.asynchronousError) owner.asynchronousError=message; }
}
static void OQCommandError(OQCatalogGraph *owner, id<MTLCommandBuffer> command) {
    if(command.status==MTLCommandBufferStatusError) OQRecordError(owner,[NSString stringWithFormat:@"catalog Metal command failed: %@",command.error.localizedDescription ?: @"unknown GPU error"]);
}
static NSArray<MPSGraphTensorData *> *OQWrappedBuffer(NSMutableDictionary *cache, id<MTLBuffer> buffer, NSArray *shape) {
    NSValue *key=[NSValue valueWithPointer:(__bridge const void *)buffer];
    NSArray *data=cache[key];
    if(!data) {
        MPSGraphTensorData *tensor=[[MPSGraphTensorData alloc] initWithMTLBuffer:buffer shape:shape dataType:MPSDataTypeFloat32];
        OQCheck(tensor!=nil,@"MPSGraph could not alias caller Metal buffer"); data=@[tensor]; cache[key]=data;
    }
    return data;
}

void *oq_catalog_graph_create(void *mtl_device, const uint8_t *json, size_t json_len,
                              const uint8_t *blob, size_t blob_len, char *error, size_t error_cap) {
    @autoreleasepool {
        if(error && error_cap) error[0]=0;
        @try {
            if(@available(macOS 13.0,*)) {} else OQFail(@"catalog GPU graphs require macOS 13 or later");
            OQCheck(mtl_device && json && json_len && (blob || !blob_len),@"invalid graph-create pointers");
            id<MTLDevice> device=(__bridge id<MTLDevice>)mtl_device;
            OQCheck(MPSSupportsMTLDevice(device),@"render Metal device does not support MPS");
            NSError *parseError=nil;
            NSDictionary *model=[NSJSONSerialization JSONObjectWithData:[NSData dataWithBytes:json length:json_len] options:0 error:&parseError];
            OQCheck([model isKindOfClass:NSDictionary.class],[NSString stringWithFormat:@"invalid model graph JSON: %@",parseError.localizedDescription ?: @"expected object"]);
            NSArray *modelInputs=model[@"inputs"], *modelOutputs=model[@"outputs"], *nodes=model[@"nodes"], *initializers=model[@"initializers"];
            OQCheck([modelInputs isKindOfClass:NSArray.class] && modelInputs.count==1 && [modelOutputs isKindOfClass:NSArray.class] && modelOutputs.count>0 && [nodes isKindOfClass:NSArray.class] && [initializers isKindOfClass:NSArray.class],@"catalog graph requires one input, output metadata, nodes and initializers");
            NSInteger opset=[model[@"opset"] integerValue]; OQCheck(opset>=11 && opset<=18,@"catalog ONNX opset must be between 11 and 18");
            NSData *constants=[NSData dataWithBytes:blob length:blob_len];
            OQCatalogGraph *owner=[OQCatalogGraph new]; owner.device=device; owner.graph=[MPSGraph new]; owner.graph.options=MPSGraphOptionsNone;
            owner.inputCache=[NSMutableDictionary dictionary]; owner.outputCache=[NSMutableDictionary dictionary];
            NSMutableDictionary<NSString *,OQCatalogValue *> *values=[NSMutableDictionary dictionary];
            for(NSDictionary *desc in initializers) { NSString *name=desc[@"name"]; OQCheck([name isKindOfClass:NSString.class] && name.length && !values[name],@"invalid/duplicate initializer name"); values[name]=OQDescriptor(desc,constants); }
            NSDictionary *input=modelInputs[0]; NSString *inputName=input[@"name"]; NSArray *inputShape=input[@"shape"];
            OQCheck([inputName isKindOfClass:NSString.class] && inputName.length && !values[inputName] && [input[@"dtype"] integerValue]==1 && inputShape.count==4 && OQCount(inputShape)>0,@"catalog input must be one concrete rank-four FP32 image");
            MPSGraphTensor *placeholder=[owner.graph placeholderWithShape:inputShape dataType:MPSDataTypeFloat32 name:inputName];
            values[inputName]=OQValue(inputShape,1,nil,placeholder); owner.inputShape=inputShape; owner.inputBytes=OQBytes(inputShape,1);
            NSUInteger nodeIndex=0;
            for(NSDictionary *node in nodes) {
                NSString *context=[NSString stringWithFormat:@"node %lu %@ (%@)",(unsigned long)nodeIndex,node[@"name"] ?: @"",node[@"op"] ?: @"?"];
                @try {
                    OQCheck([node isKindOfClass:NSDictionary.class] && [node[@"op"] isKindOfClass:NSString.class] && [node[@"inputs"] isKindOfClass:NSArray.class] && [node[@"outputs"] isKindOfClass:NSArray.class],@"invalid node schema");
                    NSMutableArray *arguments=[NSMutableArray array];
                    for(NSString *name in node[@"inputs"]) { OQCheck([name isKindOfClass:NSString.class],@"tensor input name must be a string"); if(name.length) { OQCatalogValue *v=values[name]; if(!v) OQFail(@"unbound input %@",name); [arguments addObject:v]; } else [arguments addObject:NSNull.null]; }
                    NSArray *results=OQTranslate(owner.graph,node,arguments,constants,opset);
                    NSArray *names=node[@"outputs"]; OQCheck(results.count==names.count,@"operator output arity mismatch");
                    for(NSUInteger i=0;i<names.count;++i) { NSString *name=names[i]; OQCheck([name isKindOfClass:NSString.class] && name.length && !values[name],@"invalid/duplicate node output name"); values[name]=results[i]; }
                } @catch(NSException *exception) { OQFail(@"%@: %@",context,exception.reason ?: exception.name); }
                ++nodeIndex;
            }
            NSString *selected=model[@"selected_output"];
            if(!selected) selected=[model[@"model_id"] isEqual:@"u2netp"]?@"1959":modelOutputs[0][@"name"];
            NSDictionary *output=nil; for(NSDictionary *candidate in modelOutputs) if([candidate[@"name"] isEqual:selected]) output=candidate;
            OQCheck(output!=nil,@"selected catalog output has no metadata");
            OQCatalogValue *result=values[selected]; OQCheck(result!=nil,@"selected catalog output has no producer");
            OQCheck([result.shape isEqual:output[@"shape"]] && result.dtype==[output[@"dtype"] integerValue],[NSString stringWithFormat:@"selected output %@ inferred %@ dtype %ld, expected %@ dtype %@",selected,result.shape,(long)result.dtype,output[@"shape"],output[@"dtype"]]);
            MPSGraphTensor *target=OQTensor(owner.graph,result);
            if(result.dtype!=1) target=[owner.graph castTensor:target toType:MPSDataTypeFloat32 name:@"catalog_export_fp32"];
            owner.outputShape=result.shape; owner.outputBytes=OQBytes(result.shape,1);
            OQCheck(owner.inputBytes<=device.maxBufferLength && owner.outputBytes<=device.maxBufferLength,@"catalog input/output exceeds render-device buffer limit");
            MPSGraphDevice *graphDevice=[MPSGraphDevice deviceWithMTLDevice:device];
            MPSGraphCompilationDescriptor *compilation=[MPSGraphCompilationDescriptor new]; compilation.optimizationLevel=MPSGraphOptimizationLevel0; compilation.waitForCompilationCompletion=YES;
            if(@available(macOS 26.0,*)) compilation.reducedPrecisionFastMath=MPSGraphReducedPrecisionFastMathNone;
            __block NSString *compilationError=nil;
            compilation.compilationCompletionHandler=^(MPSGraphExecutable *executable,NSError *failure) { (void)executable; if(failure) compilationError=[failure.localizedDescription copy]; };
            MPSGraphShapedType *inputType=[[MPSGraphShapedType alloc] initWithShape:inputShape dataType:MPSDataTypeFloat32];
            owner.executable=[owner.graph compileWithDevice:graphDevice feeds:@{placeholder:inputType} targetTensors:@[target] targetOperations:nil compilationDescriptor:compilation];
            if(compilationError) OQFail(@"catalog GPU compilation failed for output %@: %@",selected,compilationError);
            OQCheck(owner.executable!=nil,@"catalog GPU graph compilation produced no executable"); owner.executable.options=MPSGraphOptionsNone;
            OQCheck(owner.executable.feedTensors.count==1 && owner.executable.targetTensors.count==1,@"compiled catalog executable has unexpected input/output arity");
            return (__bridge_retained void *)owner;
        } @catch(NSException *exception) { OQError(error,error_cap,exception.reason ?: exception.name); return NULL; }
    }
}

void oq_catalog_graph_release(void *graph) {
    @autoreleasepool { @try { if(graph) { id released=CFBridgingRelease(graph); (void)released; } } @catch(NSException *exception) { (void)exception; } }
}
void *oq_catalog_buffer_create(void *mtl_device,size_t bytes,char *error,size_t error_cap) {
    @autoreleasepool {
        if(error && error_cap) error[0]=0;
        @try {
            OQCheck(mtl_device && bytes>0,@"invalid private Metal buffer allocation");
            id<MTLDevice> device=(__bridge id<MTLDevice>)mtl_device;
            OQCheck(bytes<=device.maxBufferLength,@"private Metal buffer exceeds device limit");
            id<MTLBuffer> buffer=[device newBufferWithLength:bytes options:MTLResourceStorageModePrivate|MTLResourceHazardTrackingModeTracked];
            OQCheck(buffer!=nil,@"private Metal buffer allocation failed"); buffer.label=@"OpenQuartz catalog resident tensor";
            return (__bridge_retained void *)buffer;
        } @catch(NSException *exception) { OQError(error,error_cap,exception.reason ?: exception.name); return NULL; }
    }
}
void oq_catalog_buffer_release(void *buffer) {
    @autoreleasepool { @try { if(buffer) { id released=CFBridgingRelease(buffer); (void)released; } } @catch(NSException *exception) { (void)exception; } }
}
int oq_catalog_graph_encode(void *graph,void *queue,void *input_buffer,void *output_buffer,char *error,size_t error_cap) {
    @autoreleasepool {
        if(error && error_cap) error[0]=0;
        @try {
            OQCheck(graph && queue && input_buffer && output_buffer,@"invalid graph-encode pointers");
            OQCatalogGraph *owner=(__bridge OQCatalogGraph *)graph;
            id<MTLCommandQueue> commands=(__bridge id<MTLCommandQueue>)queue;
            id<MTLBuffer> input=(__bridge id<MTLBuffer>)input_buffer, output=(__bridge id<MTLBuffer>)output_buffer;
            @synchronized(owner) {
                if(owner.asynchronousError) OQFail(@"%@",owner.asynchronousError);
                OQCheck(commands.device==owner.device && input.device==owner.device && output.device==owner.device,@"catalog execution must use the exact render Metal device and command queue");
                OQCheck(input!=output && input.length>=owner.inputBytes && output.length>=owner.outputBytes,@"catalog input/output buffers alias or are undersized");
                OQCheck(input.storageMode==MTLStorageModePrivate && output.storageMode==MTLStorageModePrivate,@"catalog frame tensors must use private Metal storage");
                OQCheck(input.hazardTrackingMode==MTLHazardTrackingModeTracked && output.hazardTrackingMode==MTLHazardTrackingModeTracked,@"catalog frame tensors require tracked Metal hazards");
                NSArray *feeds=OQWrappedBuffer(owner.inputCache,input,owner.inputShape), *results=OQWrappedBuffer(owner.outputCache,output,owner.outputShape);
                id<MTLCommandBuffer> native=[commands commandBuffer]; OQCheck(native!=nil,@"render Metal queue could not allocate command buffer"); native.label=@"OpenQuartz catalog GPU graph";
                MPSCommandBuffer *command=[MPSCommandBuffer commandBufferWithCommandBuffer:native];
                [native addCompletedHandler:^(id<MTLCommandBuffer> completed) { OQCommandError(owner,completed); }];
                MPSGraphExecutableExecutionDescriptor *execution=[MPSGraphExecutableExecutionDescriptor new]; execution.waitUntilCompleted=NO;
                execution.completionHandler=^(NSArray<MPSGraphTensorData *> *returned,NSError *failure) {
                    (void)returned; if(failure) OQRecordError(owner,[NSString stringWithFormat:@"catalog executable failed: %@",failure.localizedDescription]);
                };
                NSArray *encoded=[owner.executable encodeToCommandBuffer:command inputsArray:feeds resultsArray:results executionDescriptor:execution];
                OQCheck(encoded.count==1,@"catalog executable did not encode its selected output");
                // encode may commitAndContinue internally. Attach ownership to its
                // final root, not the already-committed original command buffer.
                id<MTLCommandBuffer> final=command.rootCommandBuffer;
                [final addCompletedHandler:^(id<MTLCommandBuffer> completed) { (void)feeds; (void)results; (void)encoded; OQCommandError(owner,completed); }];
                [command commit];
                // All graph commands are committed on the caller's queue before
                // Rust submits GPU postprocessing; no host wait or readback.
                return 1;
            }
        } @catch(NSException *exception) { OQError(error,error_cap,exception.reason ?: exception.name); return 0; }
    }
}
